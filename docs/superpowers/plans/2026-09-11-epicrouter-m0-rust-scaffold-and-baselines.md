# EpicRouter M0: Rust Workspace Scaffold + Java Golden Baselines — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Rust workspace (9 crates + parity harness per the approved design) and capture machine-verified golden baselines from the Java oracle engine for Tier A/B/C fixtures — the zero-regression foundation every later milestone gates against.

**Architecture:** The Java engine (unmodified, at baseline `e7f9bdf1`) is run headless via its existing CLI (`-de <dsn> -do <ses> --router.result_json=<json>`) which already emits a structured `RoutingResultManifest` (schema v1). A new Rust binary, `epic-harness`, orchestrates: build jar → run per fixture → distill manifest + SES hash into a committed `baseline.json` per fixture → later re-run and diff (verify) to prove oracle stability and (from M3) Rust-vs-Java parity. The 9 library crates are scaffolded now with their architectural responsibility documented, so the workspace layout from the design doc is locked in from day one.

**Tech Stack:** Rust 1.93 (edition 2024, pinned via `rust-toolchain.toml`), serde/serde_json/serde_norway (fork of serde_yaml), sha2, clap 4, anyhow. Java side: `./gradlew executableJar` → `build/libs/freerouting-current-executable.jar`, run with `~/.jdks/jdk-25.0.4.1+1/bin/java` (**system java is 21 and cannot run the jar** — class file version 69).

**Design doc:** `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` (M0 = §6 row 1; harness = §5; workspace = §4.1)

**Standing constraints (every task):**
- Never modify anything outside this repo; never touch other Claude Code instances or global `~/.claude` config.
- Never push to the `upstream` remote. All commits land on local branch `epic/main`.
- Never modify Java sources under `src/` — the oracle must stay byte-identical to `e7f9bdf1`.
- Commit messages end with `Co-Authored-By: Claude Code <noreply@anthropic.com>`.

---

## Ground truth established during planning (do not re-derive)

- **Oracle invocation** (verified in `src/main/java/app/freerouting/Freerouting.java:85-168` and `settings/GlobalSettings.java:608-770`): CLI mode triggers when both `-de <in.dsn>` and `-do <out.ses>` are present; it runs fanout → autoroute → optimizer, writes the SES, and exits. `--router.result_json=<path>` (serialized name `result_json`, `RouterSettings.java:94`) writes the manifest. `--usage_and_diagnostic_data.disable_analytics=true` opts out of telemetry.
- **Manifest schema** (`core/results/RoutingResultManifest.java`, Gson, snake_case): `schema_version`, `app_version`, `git_sha`, `fixture{filename,sha256}`, `phases{fanout,autorouter,optimizer}`, `board_statistics{connections{incomplete_count,maximum_count},clearance_violations{total_count,router_introduced_count},…}`, `normalized_score`, `optimizer_score`, `final_state`, `exit_code`, `output_written`. Gson emits fields we do not model — the Rust mirror must **not** use `deny_unknown_fields`.
- **Java version**: jar requires Java 25; system default is 21. Use `~/.jdks/jdk-25.0.4.1+1/bin/java` (exists, verified).
- **GUI interception trap** (discovered executing Task 4): with a display present and GUI enabled in `~/.freerouting/freerouting.json`, `-de/-do` batch runs are intercepted by the GUI auto-start flow — `initializeCli`, the only reader of `routerSettings.resultJsonPath` (`Freerouting.java:352-375`), never executes, and the run "succeeds" (exit 0, SES written) with NO manifest. The oracle argv therefore carries `--gui.enabled=false --api_server.enabled=false`, mirroring the repo's own baseline producer `scripts/benchmark/lib/BenchmarkRunner.ps1` (~lines 112-116). These flags are load-bearing on any machine with a display.
- **Fixture corpus**: `scripts/benchmark/fixtures/metadata.yaml` (27 fixtures with size/layer/net/timeout/tags). Four KiCad demos are disabled on disk (`.dsn_disabled` suffix, renamed by upstream `39c0f633` "Disable the benchmark fixtures that are timing out"): `kit-dev-coldfire-xilinx_5213`, `RoyalBlue54L-Feather`, `video`, `vme-wren` — **all excluded** from M0 tiers (`metadata.yaml` is stale on the last two). One fixture has a space in its name (`sonde xilinx.dsn`) — spawn processes with argv arrays, never shell strings.
- **Machine**: 16 cores, 31 GB RAM (~17 GB in use by other work). Capture runs sequentially (clean timing; other instances are running).

---

## Task 1: Rust workspace scaffold (9 crates + harness)

**Files:**
- Create: `rust/Cargo.toml`
- Create: `rust/rust-toolchain.toml`
- Create: `rust/.gitignore`
- Create: `rust/crates/epic-geometry/Cargo.toml`, `rust/crates/epic-geometry/src/lib.rs`
- Create: `rust/crates/epic-dsn/Cargo.toml`, `rust/crates/epic-dsn/src/lib.rs`
- Create: `rust/crates/epic-board/Cargo.toml`, `rust/crates/epic-board/src/lib.rs`
- Create: `rust/crates/epic-index/Cargo.toml`, `rust/crates/epic-index/src/lib.rs`
- Create: `rust/crates/epic-drc/Cargo.toml`, `rust/crates/epic-drc/src/lib.rs`
- Create: `rust/crates/epic-router/Cargo.toml`, `rust/crates/epic-router/src/lib.rs`
- Create: `rust/crates/epic-engine/Cargo.toml`, `rust/crates/epic-engine/src/lib.rs`
- Create: `rust/crates/epic-cli/Cargo.toml`, `rust/crates/epic-cli/src/lib.rs`
- Create: `rust/crates/epic-gui/Cargo.toml`, `rust/crates/epic-gui/src/lib.rs`
- Create: `rust/harness/Cargo.toml`, `rust/harness/src/main.rs`

- [ ] **Step 1: Write the workspace root files**

`rust/Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = [
    "crates/epic-geometry",
    "crates/epic-dsn",
    "crates/epic-board",
    "crates/epic-index",
    "crates/epic-drc",
    "crates/epic-router",
    "crates/epic-engine",
    "crates/epic-cli",
    "crates/epic-gui",
    "harness",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "GPL-3.0-or-later"
rust-version = "1.93"

[workspace.lints.rust]
# SIMD kernels in epic-geometry will opt in per-module with #[allow(unsafe_code)].
unsafe_code = "warn"

[workspace.lints.clippy]
unwrap_used = "warn"
```

`rust/rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.93.0"
```

`rust/.gitignore`:

```gitignore
/target
harness/runs/
```

- [ ] **Step 2: Write the 9 crate manifests**

Every library crate gets the same `Cargo.toml` shape with its own name. For `epic-geometry`:

```toml
[package]
name = "epic-geometry"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lints]
workspace = true
```

Repeat for `epic-dsn`, `epic-board`, `epic-index`, `epic-drc`, `epic-router`, `epic-engine`, `epic-cli`, `epic-gui` (only the `name` line changes). These eight have no dependencies in M0.

- [ ] **Step 3: Write the 9 lib.rs skeletons**

Each states its architectural responsibility from design §4.1 (this is the contract future milestones implement). `rust/crates/epic-geometry/src/lib.rs`:

```rust
//! Exact 45-degree integer geometry kernel: IntPoint, IntBox, IntOctagon,
//! TileShape, Polyline.
//!
//! Port discipline (design doc §9): i128 fast path with checked arithmetic and
//! BigRational fallback; no `mul_add`; operation order identical to the Java
//! `geometry/planar` package it replaces.

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M1.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-geometry");
    }
}
```

`rust/crates/epic-dsn/src/lib.rs`:

```rust
//! Specctra DSN/SES reader and writer with semantically-normalized
//! round-trip parity against the Java `io.specctra` package (design §5,
//! I/O parity gate).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M1.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-dsn");
    }
}
```

`rust/crates/epic-board/src/lib.rs`:

```rust
//! Board model: items, layers, nets, clearance classes, pours, keepouts.
//! Arena-allocated struct-of-arrays layout with snapshot/undo (design §4.1;
//! content lands in M2).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M2.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-board");
    }
}
```

`rust/crates/epic-index/src/lib.rs`:

```rust
//! Spatial index: arena BVH/quadtree with incremental insert/remove on the
//! rip-up hot path, per-clearance-class Minkowski compensation (design §4.1;
//! content lands in M2).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M2.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-index");
    }
}
```

`rust/crates/epic-drc/src/lib.rs`:

```rust
//! Incremental exact design-rule checking; the single violation authority
//! (design §4.1; content lands in M2/M6).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M2/M6.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-drc");
    }
}
```

`rust/crates/epic-router/src/lib.rs`:

```rust
//! The staged router pipeline: fanout/escape, global routing (PathFinder
//! negotiated congestion), detail routing (octagon A* + push-and-shove),
//! plane routing, tuning, gloss (design §4.2; content lands from M3 on).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands from M3 on.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-router");
    }
}
```

`rust/crates/epic-engine/src/lib.rs`:

```rust
//! Jobs, layered settings (defaults -> JSON -> DSN -> env -> CLI -> GUI with
//! Java SettingsMerger precedence semantics), progress/cancel, seed control
//! (design §4.1; content lands in M3).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M3.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-engine");
    }
}
```

`rust/crates/epic-cli/src/lib.rs`:

```rust
//! `epicrouter -de board.dsn -do out.ses` drop-in CLI (design §4.1; content
//! lands in M3).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M3.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-cli");
    }
}
```

`rust/crates/epic-gui/src/lib.rs`:

```rust
//! egui/wgpu frontend: GPU board canvas, per-stage routing progress,
//! ratsnest, DRC markers, congestion heatmap (design §7; content lands in M9).

#[cfg(test)]
mod tests {
    /// The crate is a scaffold in M0; content lands in M9.
    #[test]
    fn crate_scaffolds() {
        let name = env!("CARGO_PKG_NAME");
        assert_eq!(name, "epic-gui");
    }
}
```

- [ ] **Step 4: Write the harness crate skeleton**

`rust/harness/Cargo.toml`:

```toml
[package]
name = "epic-harness"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[[bin]]
name = "epic-harness"
path = "src/main.rs"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_norway = "0.9" # maintained drop-in fork of the archived serde_yaml (RUSTSEC-2024-0320); API-identical
sha2 = "0.10"

[lints]
workspace = true
```

`rust/harness/src/main.rs` (placeholder that Task 2 replaces):

```rust
//! Differential parity + benchmark harness for the EpicRouter rewrite
//! (design §5). M0 scope: drive the Java oracle engine and capture golden
//! baselines.

fn main() {
    println!("epic-harness: not yet implemented (see plan Task 2)");
}
```

- [ ] **Step 5: Verify the workspace builds and tests pass**

Run (from `rust/`):

```bash
cd rust && cargo test --workspace
```

Expected: 10 crates compile, 10 tests pass (one per crate). First run downloads crates.io deps for `harness`.

- [ ] **Step 6: Verify fmt and clippy are clean**

```bash
cd rust && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
```

Expected: no output, exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/
git commit -m "feat(m0): scaffold Rust workspace with 9 epic crates + parity harness skeleton

Workspace layout per design doc §4.1. Each crate skeleton documents its
architectural responsibility; all content lands in later milestones.

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 2: Tier file model + curated fixture matrix

**Files:**
- Create: `rust/harness/config/tiers.yaml`
- Create: `rust/harness/src/tiers.rs`
- Modify: `rust/harness/src/main.rs` (replace placeholder)

- [ ] **Step 1: Write the failing test first**

Create `rust/harness/src/tiers.rs` with only the types and an embedded-literal test (the load/validate functions are added in Step 3 after seeing red):

```rust
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::PathBuf;

/// The curated fixture matrix (design §5). `fixtures_root` is resolved
/// against the repository root (the harness locates it by walking up from the
/// working directory until it finds `.git` + `build.gradle`).
#[derive(Debug, Deserialize)]
pub struct TierFile {
    pub fixtures_root: PathBuf,
    pub tiers: Vec<Tier>,
}

#[derive(Debug, Deserialize)]
pub struct Tier {
    pub name: String,
    pub fixtures: Vec<FixtureRef>,
}

#[derive(Debug, Deserialize)]
pub struct FixtureRef {
    /// Path relative to `fixtures_root`. May contain spaces (e.g.
    /// `KiCad_10_demos/sonde xilinx.dsn`).
    pub path: String,
    pub timeout_seconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
fixtures_root: scripts/benchmark/fixtures
tiers:
  - name: A
    fixtures:
      - path: DAC2020_boards/DAC2020_bm08.dsn
        timeout_seconds: 120
      - path: KiCad_10_demos/sonde xilinx.dsn
        timeout_seconds: 300
  - name: C
    fixtures:
      - path: KiCad_10_demos/video.dsn
        timeout_seconds: 1800
"#;

    #[test]
    fn parses_tiers_and_preserves_spaces_in_paths() {
        let file: TierFile = serde_norway::from_str(SAMPLE).expect("sample must parse");
        assert_eq!(file.fixtures_root, PathBuf::from("scripts/benchmark/fixtures"));
        assert_eq!(file.tiers.len(), 2);
        let a = file.tier("A").expect("tier A");
        assert_eq!(a.fixtures.len(), 2);
        assert_eq!(a.fixtures[1].path, "KiCad_10_demos/sonde xilinx.dsn");
        assert_eq!(a.fixtures[1].timeout_seconds, 300);
        assert!(file.tier("B").is_none());
    }

    #[test]
    fn validation_rejects_zero_timeout_and_empty_tier() {
        let bad: TierFile =
            serde_norway::from_str("fixtures_root: x\ntiers:\n  - name: A\n    fixtures:\n      - path: a.dsn\n        timeout_seconds: 0\n")
                .expect("structurally valid");
        assert!(bad.validate().is_err());

        let empty: TierFile = serde_norway::from_str(
            "fixtures_root: x\ntiers:\n  - name: A\n    fixtures: []\n",
        )
        .expect("structurally valid");
        assert!(empty.validate().is_err());
    }
}
```

Add the module to `rust/harness/src/main.rs`:

```rust
mod tiers;

fn main() {
    println!("epic-harness: tiers module loaded");
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

```bash
cd rust && cargo test -p epic-harness tiers
```

Expected: compile error — `validate` and `tier` not yet defined on `TierFile`.

- [ ] **Step 3: Implement `validate`, `tier`, and `load`**

Append inside `tiers.rs` (after the struct definitions, before `mod tests`):

```rust
impl TierFile {
    /// Loads and validates the tier file at `path`.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading tier file {}", path.display()))?;
        let file: TierFile = serde_norway::from_str(&raw)
            .with_context(|| format!("parsing tier file {}", path.display()))?;
        file.validate()?;
        Ok(file)
    }

    /// Structural sanity: non-empty everywhere, positive timeouts.
    pub fn validate(&self) -> Result<()> {
        if self.tiers.is_empty() {
            bail!("tier file contains no tiers");
        }
        for tier in &self.tiers {
            if tier.name.is_empty() {
                bail!("tier with empty name");
            }
            if tier.fixtures.is_empty() {
                bail!("tier {} has no fixtures", tier.name);
            }
            for fixture in &tier.fixtures {
                if fixture.timeout_seconds == 0 {
                    bail!("fixture {} has timeout_seconds 0", fixture.path);
                }
            }
        }
        Ok(())
    }

    /// Case-insensitive tier lookup by name.
    pub fn tier(&self, name: &str) -> Option<&Tier> {
        self.tiers.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cd rust && cargo test -p epic-harness
```

Expected: 2 passed.

- [ ] **Step 5: Write the curated tier matrix**

`rust/harness/config/tiers.yaml` — 23 fixtures drawn from `scripts/benchmark/fixtures` per its own `metadata.yaml` tags (canary → A; routine 2-layer → B; multi-layer/large → C). The four `.dsn_disabled` KiCad demos are excluded (kit-dev-coldfire, RoyalBlue54L, video, vme-wren — the last two despite stale metadata.yaml entries). Timeout budgets come straight from `metadata.yaml`:

```yaml
# EpicRouter parity fixture matrix (M0).
# fixtures_root is resolved against the repository root, not this file.
# Tier A: canary gate — fast boards, must stay clean (design §5).
# Tier B: routine 2-4 layer boards.
# Tier C: complex / multi-layer / large boards.
# Excluded: the four .dsn_disabled KiCad demos (kit-dev-coldfire-xilinx_5213,
# RoyalBlue54L-Feather, video, vme-wren — disabled upstream for timing out).
fixtures_root: scripts/benchmark/fixtures
tiers:
  - name: A
    fixtures:
      - { path: DAC2020_boards/DAC2020_bm01.dsn, timeout_seconds: 1800 }
      - { path: DAC2020_boards/DAC2020_bm02.dsn, timeout_seconds: 300 }
      - { path: DAC2020_boards/DAC2020_bm06.dsn, timeout_seconds: 600 }
      - { path: DAC2020_boards/DAC2020_bm07.dsn, timeout_seconds: 300 }
      - { path: DAC2020_boards/DAC2020_bm08.dsn, timeout_seconds: 120 }
      - { path: DAC2020_boards/DAC2020_bm09.dsn, timeout_seconds: 600 }
      - { path: DAC2020_boards/DAC2020_bm11.dsn, timeout_seconds: 600 }
      - { path: KiCad_10_demos/ecc83-pp.dsn, timeout_seconds: 120 }
      - { path: KiCad_10_demos/ecc83-pp_v2.dsn, timeout_seconds: 120 }
      - { path: KiCad_10_demos/pic_programmer.dsn, timeout_seconds: 300 }
      - { path: "KiCad_10_demos/sonde xilinx.dsn", timeout_seconds: 300 }
  - name: B
    fixtures:
      - { path: DAC2020_boards/DAC2020_bm05.dsn, timeout_seconds: 1800 }
      - { path: DAC2020_boards/DAC2020_bm10.dsn, timeout_seconds: 900 }
      - { path: KiCad_10_demos/complex_hierarchy.dsn, timeout_seconds: 600 }
      - { path: KiCad_10_demos/interf_u.dsn, timeout_seconds: 1800 }
      - { path: KiCad_10_demos/multichannel_mixer.dsn, timeout_seconds: 600 }
      - { path: KiCad_10_demos/multichannel_mixer-unrouted.dsn, timeout_seconds: 900 }
      - { path: KiCad_10_demos/StickHub.dsn, timeout_seconds: 900 }
      - { path: PCBench/1-Wire-Wing-pcb_1-Wire_Wing/unrouted.dsn, timeout_seconds: 300 }
      - { path: PCBench/1Bitsy_1bitsy/unrouted.dsn, timeout_seconds: 300 }
  - name: C
    fixtures:
      - { path: DAC2020_boards/DAC2020_bm04.dsn, timeout_seconds: 2700 }
      - { path: KiCad_10_demos/CM5_MINIMA_3.dsn, timeout_seconds: 1800 }
      - { path: PCBench/front-end-modules_LimeSDR_Sony/unrouted.dsn, timeout_seconds: 900 }
```

- [ ] **Step 6: Verify every referenced fixture exists on disk**

```bash
cd rust && python3 - <<'EOF'
import yaml, pathlib
root = pathlib.Path('..')
cfg = yaml.safe_load(open('harness/config/tiers.yaml'))
base = root / cfg['fixtures_root']
missing = []
for tier in cfg['tiers']:
    for f in tier['fixtures']:
        p = base / f['path']
        if not p.is_file():
            missing.append(f"{tier['name']}: {f['path']}")
print("missing:", missing if missing else "none")
assert not missing
EOF
```

Expected: `missing: none`. (If a path with a space fails here, fix the YAML quoting, not the checker.)

- [ ] **Step 7: Commit**

```bash
git add rust/harness/config/tiers.yaml rust/harness/src/
git commit -m "feat(m0): tier-file model + curated parity fixture matrix

Tiers mirror scripts/benchmark/fixtures/metadata.yaml tags (canary->A,
routine->B, multilayer/large->C); the two .dsn_disabled demos are excluded.

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 3: Java manifest mirror (`manifest.rs`)

**Files:**
- Create: `rust/harness/src/manifest.rs`
- Modify: `rust/harness/src/main.rs` (add `mod manifest;`)

- [ ] **Step 1: Write the failing test with a realistic manifest literal**

`rust/harness/src/manifest.rs`:

```rust
//! Serde mirror of the Java `RoutingResultManifest` (schema v1) — the subset
//! the parity gates read. Field names MUST match the `@SerializedName`
//! values in `src/main/java/app/freerouting/core/results/RoutingResultManifest.java`.
//! Gson emits additional fields (bounds, resource_usage, settings_snapshot…)
//! that we deliberately ignore: do NOT add `deny_unknown_fields`.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RoutingResultManifest {
    pub schema_version: u32,
    pub app_version: String,
    pub git_sha: String,
    pub fixture: FixtureInfo,
    pub phases: PhaseMetrics,
    pub board_statistics: Option<BoardStatistics>,
    pub normalized_score: Option<f64>,
    pub optimizer_score: Option<f64>,
    pub final_state: String,
    pub exit_code: i32,
    pub output_written: bool,
}

#[derive(Debug, Deserialize)]
pub struct FixtureInfo {
    pub filename: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PhaseMetrics {
    #[serde(default)]
    pub fanout: PhaseDetail,
    pub autorouter: PhaseDetail,
    pub optimizer: PhaseDetail,
}

#[derive(Debug, Default, Deserialize)]
pub struct PhaseDetail {
    pub duration_seconds: Option<f64>,
    pub passes_completed: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct BoardStatistics {
    pub connections: Option<Connections>,
    pub clearance_violations: Option<ClearanceViolations>,
}

#[derive(Debug, Deserialize)]
pub struct Connections {
    pub incomplete_count: Option<i64>,
    pub maximum_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ClearanceViolations {
    pub total_count: Option<i64>,
    pub router_introduced_count: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "schema_version": 1,
  "generated_at": "2026-09-11T12:00:00Z",
  "app_version": "2.5.0",
  "git_sha": "e7f9bdf1",
  "fixture": { "filename": "DAC2020_bm08.dsn", "sha256": "abc123" },
  "settings_snapshot": { "ignored": true },
  "phases": {
    "fanout": { "duration_seconds": 0.5, "passes_completed": 1 },
    "autorouter": { "duration_seconds": 12.25, "passes_completed": 20 },
    "optimizer": { "duration_seconds": 30.5, "passes_completed": 5 }
  },
  "board_statistics": {
    "connections": { "incomplete_count": 0, "maximum_count": 42 },
    "clearance_violations": { "total_count": 0, "router_introduced_count": 0 },
    "vias": { "total_count": 10 }
  },
  "bounds": { "ignored": true },
  "normalized_score": 998.5,
  "optimizer_score": 991.25,
  "final_state": "COMPLETED",
  "exit_code": 0,
  "output_written": true,
  "cpu_score": null
}"#;

    #[test]
    fn parses_manifest_ignoring_unknown_fields() {
        let m: RoutingResultManifest =
            serde_json::from_str(SAMPLE).expect("manifest must parse");
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.git_sha, "e7f9bdf1");
        assert_eq!(m.fixture.sha256.as_deref(), Some("abc123"));
        assert_eq!(m.phases.autorouter.duration_seconds, Some(12.25));
        assert_eq!(m.phases.autorouter.passes_completed, Some(20));
        let stats = m.board_statistics.expect("stats present");
        assert_eq!(stats.connections.expect("connections").incomplete_count, Some(0));
        assert_eq!(
            stats.clearance_violations.expect("violations").total_count,
            Some(0)
        );
        assert_eq!(m.normalized_score, Some(998.5));
        assert_eq!(m.final_state, "COMPLETED");
        assert_eq!(m.exit_code, 0);
        assert!(m.output_written);
    }
}
```

Add `mod manifest;` to `main.rs` (keep `mod tiers;`).

- [ ] **Step 2: Run the test**

```bash
cd rust && cargo test -p epic-harness manifest
```

Expected: 1 passed. (The types and test are written together here because the test exercises serde derive on the same declaration — there is no separate "minimal implementation" step; if it fails, fix the struct/field mapping until green.)

- [ ] **Step 3: Commit**

```bash
git add rust/harness/src/
git commit -m "feat(m0): mirror Java RoutingResultManifest schema v1 in harness

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 4: Oracle runner (`oracle.rs`)

**Files:**
- Create: `rust/harness/src/oracle.rs`
- Modify: `rust/harness/src/main.rs` (add `mod oracle;`)

- [ ] **Step 1: Write the failing tests**

`rust/harness/src/oracle.rs` starts with the pure logic under test (argv construction) plus tests:

```rust
//! Drives the Java engine as the parity oracle: builds the executable jar,
//! runs headless CLI jobs per fixture, and returns their manifests.
//! The Java tree is never modified (design §4.1).

use crate::manifest::RoutingResultManifest;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where results of one oracle invocation land.
#[derive(Debug)]
pub struct OracleRun {
    pub manifest: Option<RoutingResultManifest>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub wall_seconds: f64,
    pub ses_path: PathBuf,
    pub manifest_path: PathBuf,
}

pub fn find_repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("getting current directory")?;
    loop {
        if dir.join(".git").exists() && dir.join("build.gradle").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("could not locate repo root (no .git + build.gradle upward from cwd)");
        }
    }
}

/// The jar requires Java 25; the system default may be older, so prefer an
/// explicit override, then `~/.jdks/jdk-25*`, then PATH `java`.
pub fn resolve_java() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("EPIC_ORACLE_JAVA") {
        return Ok(PathBuf::from(explicit));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let jdk_root = Path::new(&home).join(".jdks");
        let mut candidates: Vec<PathBuf> = match std::fs::read_dir(&jdk_root) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("jdk-25"))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        candidates.sort();
        if let Some(jdk) = candidates.first() {
            let java = jdk.join("bin").join("java");
            if java.is_file() {
                return Ok(java);
            }
        }
    }
    Ok(PathBuf::from("java"))
}

pub fn jar_path(repo_root: &Path) -> PathBuf {
    repo_root.join("build/libs/freerouting-current-executable.jar")
}

/// Builds the oracle jar via Gradle unless `EPIC_SKIP_GRADLE=1`.
pub fn build_jar(repo_root: &Path) -> Result<()> {
    if std::env::var("EPIC_SKIP_GRADLE").as_deref() == Ok("1") {
        return Ok(());
    }
    let status = Command::new("./gradlew")
        .arg("-q")
        .arg("executableJar")
        .current_dir(repo_root)
        .status()
        .context("running ./gradlew executableJar")?;
    if !status.success() {
        bail!("gradle executableJar failed with {status}");
    }
    Ok(())
}

/// Pure argv builder (unit-tested; no I/O). Paths may contain spaces —
/// callers pass this directly to `Command::args`, never through a shell.
pub fn oracle_argv(
    java: &Path,
    jar: &Path,
    jvm_xmx: &str,
    dsn: &Path,
    ses_out: &Path,
    manifest_out: &Path,
) -> Vec<String> {
    vec![
        java.to_string_lossy().into_owned(),
        format!("-Xmx{jvm_xmx}"),
        "-jar".into(),
        jar.to_string_lossy().into_owned(),
        "-de".into(),
        dsn.to_string_lossy().into_owned(),
        "-do".into(),
        ses_out.to_string_lossy().into_owned(),
        format!("--router.result_json={}", manifest_out.to_string_lossy()),
        "--usage_and_diagnostic_data.disable_analytics=true".into(),
        // Only the headless CLI path (`initializeCli`) honors
        // `--router.result_json`; with a display present, the GUI auto-start
        // batch flow would intercept `-de/-do` and never write the manifest.
        // Mirror scripts/benchmark/lib/BenchmarkRunner.ps1.
        "--gui.enabled=false".into(),
        "--api_server.enabled=false".into(),
    ]
}

/// Spawns one oracle run. `work_dir` receives `out.ses`, `manifest.json`,
/// `stdout.log`, `stderr.log`. Output goes to files (not pipes) so a chatty
/// JVM can never deadlock on a full pipe buffer.
pub fn run_oracle(
    java: &Path,
    jar: &Path,
    dsn: &Path,
    work_dir: &Path,
    timeout: Duration,
    jvm_xmx: &str,
) -> Result<OracleRun> {
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("creating work dir {}", work_dir.display()))?;
    let ses_path = work_dir.join("out.ses");
    let manifest_path = work_dir.join("manifest.json");
    let stdout_path = work_dir.join("stdout.log");
    let stderr_path = work_dir.join("stderr.log");

    let argv = oracle_argv(java, jar, jvm_xmx, dsn, &ses_path, &manifest_path);
    let (java_bin, rest) = argv.split_first().expect("argv is never empty");
    let stdout_file = std::fs::File::create(&stdout_path)
        .with_context(|| format!("creating {}", stdout_path.display()))?;
    let stderr_file = std::fs::File::create(&stderr_path)
        .with_context(|| format!("creating {}", stderr_path.display()))?;

    let started = Instant::now();
    let mut child = Command::new(java_bin)
        .args(rest)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .with_context(|| format!("spawning oracle {}", java_bin))?;

    let (exit_code, timed_out) = wait_with_timeout(&mut child, timeout);
    let wall_seconds = started.elapsed().as_secs_f64();

    let manifest = if manifest_path.is_file() {
        serde_json::from_str(&std::fs::read_to_string(&manifest_path)?).ok()
    } else {
        None
    };

    Ok(OracleRun {
        manifest,
        exit_code,
        timed_out,
        wall_seconds,
        ses_path,
        manifest_path,
    })
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> (Option<i32>, bool) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false),
            Ok(None) => {}
            Err(_) => return (None, false),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return (None, true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_order_matches_java_cli_contract() {
        let argv = oracle_argv(
            Path::new("/jdks/jdk-25/bin/java"),
            Path::new("/repo/build/libs/freerouting-current-executable.jar"),
            "4g",
            Path::new("/repo/fixtures/sonde xilinx.dsn"),
            Path::new("/tmp/w/out.ses"),
            Path::new("/tmp/w/manifest.json"),
        );
        assert_eq!(argv[0], "/jdks/jdk-25/bin/java");
        assert_eq!(argv[1], "-Xmx4g");
        assert_eq!(argv[2], "-jar");
        assert!(argv[3].ends_with("freerouting-current-executable.jar"));
        assert_eq!(argv[4], "-de");
        assert_eq!(argv[5], "/repo/fixtures/sonde xilinx.dsn");
        assert_eq!(argv[6], "-do");
        assert_eq!(argv[7], "/tmp/w/out.ses");
        assert_eq!(
            argv[8],
            "--router.result_json=/tmp/w/manifest.json"
        );
        assert_eq!(
            argv[9],
            "--usage_and_diagnostic_data.disable_analytics=true"
        );
        assert_eq!(argv[10], "--gui.enabled=false");
        assert_eq!(argv[11], "--api_server.enabled=false");
        assert_eq!(argv.len(), 12);
    }

    /// End-to-end smoke on the tiniest fixture. Requires the jar + JDK 25;
    /// run explicitly with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn oracle_routes_bm08_end_to_end() {
        let root = find_repo_root().expect("repo root");
        build_jar(&root).expect("building jar");
        let java = resolve_java().expect("resolving java");
        let jar = jar_path(&root);
        let dsn = root
            .join("scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn");
        let work = std::env::temp_dir().join("epic-harness-bm08-smoke");
        let run = run_oracle(
            &java,
            &jar,
            &dsn,
            &work,
            Duration::from_secs(300),
            "4g",
        )
        .expect("oracle run");
        let m = run.manifest.as_ref().expect("manifest must be written");
        assert_eq!(m.final_state, "COMPLETED");
        assert_eq!(m.exit_code, 0);
        assert!(run.ses_path.is_file());
        assert!(run.ses_path.metadata().expect("ses stat").len() > 0);
    }
}
```

- [ ] **Step 2: Run unit tests, then the ignored end-to-end smoke**

```bash
cd rust && cargo test -p epic-harness oracle
```

Expected: 1 passed, 1 filtered (the ignored one).

```bash
cd rust && cargo test -p epic-harness oracle -- --ignored --nocapture
```

Expected: 1 passed after Gradle builds the jar (~1-2 min) and Java routes bm08 (15 nets, typically <60 s). If this fails, diagnose from `rust/…/epic-harness-bm08-smoke/stderr.log` before continuing — every later task depends on the oracle path working.

- [ ] **Step 3: Commit**

```bash
git add rust/harness/src/
git commit -m "feat(m0): oracle runner — build jar, run Java CLI headless, parse manifest

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 5: Baseline capture (`baseline.rs` + `capture` subcommand)

**Files:**
- Create: `rust/harness/src/baseline.rs`
- Modify: `rust/harness/src/main.rs` (wire subcommands)
- Modify: `rust/harness/src/tiers.rs`, `rust/harness/config/tiers.yaml` (Step 0 hardening)

- [ ] **Step 0: Harden tiers.rs (Task 2 quality-review findings) — separate commit first**

Apply in `rust/harness/src/tiers.rs`:
1. In `validate()`: reject duplicate tier names and duplicate fixture paths (HashSet, ~5 lines), reject absolute fixture paths (`Path::new(&fixture.path).is_relative()` must hold), and include the tier index in the "empty name" bail message; in `load()`, wrap `file.validate()` with `.with_context(|| format!("validating tier file {}", path.display()))?`.
2. Add `assert!(file.tier("a").is_some());` to the lookup test (doc comment claims case-insensitivity; test it).
3. Import `Path` and use it instead of the fully-qualified `&std::path::Path` in `load`'s signature.

In `rust/harness/config/tiers.yaml`, name the four excluded `.dsn_disabled` demos explicitly in the header comment (kit-dev-coldfire-xilinx_5213, RoyalBlue54L-Feather, video, vme-wren).

Run `cargo test -p epic-harness && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings` from `rust/`, then commit:

```bash
git add rust/harness/src/tiers.rs rust/harness/config/tiers.yaml
git commit -m "refactor(m0): harden tier-file validation (duplicates, absolute paths, error context)

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

Note for Step 3: when `main.rs`'s `run()` starts calling `TierFile::load`, DELETE the `#![allow(dead_code)]` at the top of `tiers.rs` — every item becomes live and the allow must not silently mask future dead code.

- [ ] **Step 1: Write the failing tests for `distill` and `sha256_file`**

`rust/harness/src/baseline.rs`:

```rust
//! Distills oracle runs into committed golden baselines and compares runs
//! against them (design §5: counts/score/state are gated; wall time is a
//! tracked trend, never a gate; the SES hash is identity info only —
//! semantic SES comparison arrives with epic-dsn in M1).

use crate::manifest::RoutingResultManifest;
use crate::oracle::OracleRun;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const BASELINE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineRecord {
    pub schema_version: u32,
    /// "java" in M0; "rust" records appear from M3.
    pub engine: String,
    pub app_version: Option<String>,
    pub git_sha: Option<String>,
    /// Repo-relative fixture path (e.g. `scripts/benchmark/fixtures/.../x.dsn`).
    pub fixture: String,
    pub fixture_sha256: Option<String>,
    pub final_state: String,
    pub exit_code: Option<i32>,
    pub incomplete_count: Option<i64>,
    pub maximum_count: Option<i64>,
    pub clearance_violations_total: Option<i64>,
    pub clearance_router_introduced: Option<i64>,
    pub normalized_score: Option<f64>,
    pub optimizer_score: Option<f64>,
    pub ses_sha256: Option<String>,
    pub autorouter_seconds: Option<f64>,
    pub optimizer_seconds: Option<f64>,
    pub passes_completed: Option<i64>,
    pub captured_at_unix: i64,
    pub notes: Option<String>,
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

/// Reduces one oracle run to the committed baseline record.
pub fn distill(fixture_rel: &str, run: &OracleRun) -> BaselineRecord {
    let empty_stats = Default::default();
    let (manifest, note): (&RoutingResultManifest, Option<String>) = match &run.manifest {
        Some(m) => (m, None),
        None => (
            // A missing manifest is itself a recordable outcome (harness
            // timeout, JVM crash). Synthesize the shell of a manifest; the
            // note distinguishes absent from present-but-unparseable
            // (oracle.rs `manifest_error`: schema drift or a truncated
            // mid-kill write must not read as "never produced").
            &UNPARSED,
            Some(match &run.manifest_error {
                Some(err) => format!(
                    "manifest unparseable: {err} (timed_out={}, exit_code={:?})",
                    run.timed_out, run.exit_code
                ),
                None => format!(
                    "no manifest produced (timed_out={}, exit_code={:?})",
                    run.timed_out, run.exit_code
                ),
            }),
        ),
    };
    let stats = manifest.board_statistics.as_ref();
    BaselineRecord {
        schema_version: BASELINE_SCHEMA_VERSION,
        engine: "java".into(),
        app_version: Some(manifest.app_version.clone()),
        git_sha: Some(manifest.git_sha.clone()),
        fixture: fixture_rel.into(),
        fixture_sha256: manifest.fixture.sha256.clone(),
        final_state: if run.timed_out {
            "HARNESS_TIMED_OUT".into()
        } else {
            manifest.final_state.clone()
        },
        exit_code: run.exit_code,
        incomplete_count: stats
            .and_then(|s| s.connections.as_ref())
            .and_then(|c| c.incomplete_count),
        maximum_count: stats
            .and_then(|s| s.connections.as_ref())
            .and_then(|c| c.maximum_count),
        clearance_violations_total: stats
            .and_then(|s| s.clearance_violations.as_ref())
            .and_then(|c| c.total_count),
        clearance_router_introduced: stats
            .and_then(|s| s.clearance_violations.as_ref())
            .and_then(|c| c.router_introduced_count),
        normalized_score: manifest.normalized_score,
        optimizer_score: manifest.optimizer_score,
        ses_sha256: if run.ses_path.is_file() {
            sha256_file(&run.ses_path).ok()
        } else {
            None
        },
        autorouter_seconds: manifest.phases.autorouter.duration_seconds,
        optimizer_seconds: manifest.phases.optimizer.duration_seconds,
        passes_completed: manifest.phases.autorouter.passes_completed,
        captured_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        notes: note,
    }
}

static UNPARSED: RoutingResultManifest = RoutingResultManifest {
    schema_version: 0,
    app_version: String::new(),
    git_sha: String::new(),
    fixture: crate::manifest::FixtureInfo {
        filename: None,
        sha256: None,
    },
    phases: crate::manifest::PhaseMetrics {
        fanout: crate::manifest::PhaseDetail {
            duration_seconds: None,
            passes_completed: None,
        },
        autorouter: crate::manifest::PhaseDetail {
            duration_seconds: None,
            passes_completed: None,
        },
        optimizer: crate::manifest::PhaseDetail {
            duration_seconds: None,
            passes_completed: None,
        },
    },
    board_statistics: None,
    normalized_score: None,
    optimizer_score: None,
    final_state: String::new(),
    exit_code: -1,
    output_written: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest_json() -> String {
        r#"{
          "schema_version": 1,
          "app_version": "2.5.0",
          "git_sha": "e7f9bdf1",
          "fixture": { "filename": "x.dsn", "sha256": "deadbeef" },
          "phases": {
            "fanout": {},
            "autorouter": { "duration_seconds": 10.0, "passes_completed": 20 },
            "optimizer": { "duration_seconds": 5.0, "passes_completed": 3 }
          },
          "board_statistics": {
            "connections": { "incomplete_count": 2, "maximum_count": 50 },
            "clearance_violations": { "total_count": 1, "router_introduced_count": 1 }
          },
          "normalized_score": 990.0,
          "optimizer_score": 980.0,
          "final_state": "COMPLETED",
          "exit_code": 0,
          "output_written": true
        }"#
        .into()
    }

    fn synthetic_run(manifest: Option<RoutingResultManifest>, timed_out: bool) -> OracleRun {
        let dir = std::env::temp_dir().join("epic-harness-baseline-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        OracleRun {
            manifest,
            manifest_error: None,
            exit_code: if timed_out { None } else { Some(0) },
            timed_out,
            wall_seconds: 42.0,
            ses_path: dir.join("nonexistent.ses"),
            manifest_path: dir.join("manifest.json"),
        }
    }

    #[test]
    fn distills_counts_scores_and_phases() {
        let m: RoutingResultManifest =
            serde_json::from_str(&sample_manifest_json()).expect("manifest parses");
        let run = synthetic_run(Some(m), false);
        let b = distill("fixtures/x.dsn", &run);
        assert_eq!(b.schema_version, 1);
        assert_eq!(b.engine, "java");
        assert_eq!(b.final_state, "COMPLETED");
        assert_eq!(b.incomplete_count, Some(2));
        assert_eq!(b.maximum_count, Some(50));
        assert_eq!(b.clearance_violations_total, Some(1));
        assert_eq!(b.normalized_score, Some(990.0));
        assert_eq!(b.autorouter_seconds, Some(10.0));
        assert_eq!(b.passes_completed, Some(20));
        assert_eq!(b.ses_sha256, None); // ses file does not exist in test
    }

    #[test]
    fn records_harness_timeout_honestly() {
        let run = synthetic_run(None, true);
        let b = distill("fixtures/x.dsn", &run);
        assert_eq!(b.final_state, "HARNESS_TIMED_OUT");
        assert!(b.notes.as_deref().expect("note present").contains("timed_out=true"));
        assert_eq!(b.incomplete_count, None);
    }

    #[test]
    fn records_manifest_parse_failure_honestly() {
        let mut run = synthetic_run(None, false);
        run.manifest_error = Some("expected value at line 1 column 1".into());
        let b = distill("fixtures/x.dsn", &run);
        let note = b.notes.as_deref().expect("note present");
        assert!(note.contains("manifest unparseable"));
        assert!(note.contains("expected value"));
        assert!(!note.contains("no manifest produced"));
    }

    #[test]
    fn sha256_of_known_bytes_is_stable() {
        let dir = std::env::temp_dir().join("epic-harness-baseline-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let p = dir.join("known.txt");
        std::fs::write(&p, b"epic").expect("write");
        let first = sha256_file(&p).expect("hash");
        let second = sha256_file(&p).expect("hash again");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail, then implement**

```bash
cd rust && cargo test -p epic-harness baseline
```

Expected initially: compile error (`UNPARSED` static requires `String::new()` in const context — `String::new()` IS const-stable since Rust 1.39, so this compiles; if the compiler rejects the static for another reason, replace `static UNPARSED` with a `fn unparsed() -> RoutingResultManifest` constructor and call it in `distill`). Then run again: 4 passed.

- [ ] **Step 3: Wire the CLI (`main.rs` full replacement)**

```rust
//! Differential parity + benchmark harness for the EpicRouter rewrite
//! (design §5). M0 scope: drive the Java oracle engine and capture golden
//! baselines.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod baseline;
mod manifest;
mod oracle;
mod tiers;

use oracle::OracleRun;
use tiers::TierFile;

#[derive(Parser)]
#[command(name = "epic-harness", about = "EpicRouter parity + benchmark harness")]
struct Cli {
    /// Path to the tier file (relative to cwd or absolute).
    #[arg(long, global = true, default_value = "harness/config/tiers.yaml")]
    tiers: PathBuf,

    /// JVM -Xmx for oracle runs.
    #[arg(long, global = true, default_value = "4g")]
    jvm_xmx: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the tier matrix and verify every fixture resolves on disk.
    List,
    /// Run the Java oracle on fixtures and write golden baselines.
    Capture {
        /// Only capture this tier (A/B/C); omit for all.
        #[arg(long)]
        tier: Option<String>,
    },
    /// Re-run the oracle and compare against committed baselines.
    Verify {
        /// Only verify this tier; omit for all.
        #[arg(long)]
        tier: Option<String>,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let repo_root = oracle::find_repo_root()?;
    let tiers_path = resolve_tiers_path(&repo_root, &cli.tiers);
    let tier_file = TierFile::load(&tiers_path)?;
    let fixtures_root = repo_root.join(&tier_file.fixtures_root);
    let java = oracle::resolve_java()?;
    let jar = oracle::jar_path(&repo_root);

    match cli.command {
        Command::List => {
            for tier in &tier_file.tiers {
                println!("tier {} ({} fixtures):", tier.name, tier.fixtures.len());
                for f in &tier.fixtures {
                    let p = fixtures_root.join(&f.path);
                    let status = if p.is_file() { "ok" } else { "MISSING" };
                    println!("  [{status}] {} (timeout {}s)", f.path, f.timeout_seconds);
                }
            }
            Ok(())
        }
        Command::Capture { tier } => {
            oracle::build_jar(&repo_root)?;
            let baselines_root = tiers_path
                .parent()
                .and_then(Path::parent)
                .map(|p| p.join("baselines"))
                .unwrap_or_else(|| PathBuf::from("baselines"));
            let selected = select_tiers(&tier_file, tier.as_deref())?;
            for tier in selected {
                capture_tier(
                    &tier,
                    &fixtures_root,
                    &java,
                    &jar,
                    &baselines_root,
                    &cli.jvm_xmx,
                )?;
            }
            Ok(())
        }
        Command::Verify { .. } => {
            bail!("verify is implemented in Task 6");
        }
    }
}

fn resolve_tiers_path(repo_root: &Path, given: &Path) -> PathBuf {
    if given.is_absolute() {
        given.into()
    } else if given.is_file() {
        std::fs::canonicalize(given).unwrap_or_else(|_| given.into())
    } else {
        // Default value is relative to rust/harness; also support running
        // from rust/ or repo root.
        let candidates = [
            repo_root.join("rust").join(given),
            repo_root.join(given),
        ];
        candidates
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| repo_root.join("rust").join(given))
    }
}

fn select_tiers<'a>(file: &'a TierFile, filter: Option<&str>) -> Result<Vec<&'a tiers::Tier>> {
    match filter {
        Some(name) => {
            let t = file
                .tier(name)
                .with_context(|| format!("tier {name} not found in tier file"))?;
            Ok(vec![t])
        }
        None => Ok(file.tiers.iter().collect()),
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_tier(
    tier: &tiers::Tier,
    fixtures_root: &Path,
    java: &Path,
    jar: &Path,
    baselines_root: &Path,
    jvm_xmx: &str,
) -> Result<()> {
    let run_root = baselines_root
        .parent()
        .map(|p| p.join("runs"))
        .unwrap_or_else(|| PathBuf::from("runs"));
    for fixture in &tier.fixtures {
        let dsn = fixtures_root.join(&fixture.path);
        if !dsn.is_file() {
            bail!("fixture missing: {}", dsn.display());
        }
        let work_dir = run_root.join(&tier.name).join(&fixture.path);
        println!(
            "[tier {}] routing {} (timeout {}s)…",
            tier.name,
            fixture.path,
            fixture.timeout_seconds
        );
        let run: OracleRun = oracle::run_oracle(
            java,
            jar,
            &dsn,
            &work_dir,
            Duration::from_secs(fixture.timeout_seconds),
            jvm_xmx,
        )?;
        let record = baseline::distill(&fixture.path, &run);
        let out = baselines_root
            .join("java")
            .join(&tier.name)
            .join(format!("{}.baseline.json", fixture.path));
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&out, serde_json::to_string_pretty(&record)?)
            .with_context(|| format!("writing {}", out.display()))?;
        println!(
            "  -> {} state={} incomplete={:?} violations={:?} score={:?} ({:.1}s)",
            out.display(),
            record.final_state,
            record.incomplete_count,
            record.clearance_violations_total,
            record.normalized_score,
            run.wall_seconds
        );
    }
    Ok(())
}
```

Note (amended after Task 5 quality review): `distill`'s `fixture` field stores the **repo-relative** fixture path — the raw yaml `fixtures_root` (from `tier_file`, NOT main.rs's absolute joined variable) joined with the tier entry, e.g. `scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn` — matching the `BaselineRecord.fixture` doc, so a baseline file is self-describing against the repo root. The baseline file's position under `baselines/java/<tier>/` still carries the tier. Additionally (same review): `capture` validates tier names BEFORE `build_jar`; the baselines/runs path fallbacks anchor to the tier file's directory (not cwd); and `capture` tallies infra-signature runs (`manifest.is_none() && !timed_out && exit_code != Some(0)`) — after the tier loop it prints a WARNING and returns Err (exit 1). The record file is still written (honesty preserved); routing outcomes (timeout, incomplete, violations) remain data, never errors. `sha256_file` errors fold into the record's `notes` when the SES exists but hashing fails.

- [ ] **Step 4: Run tests + smoke the CLI**

```bash
cd rust && cargo test -p epic-harness && cargo run -p epic-harness -- list
```

Expected: all unit tests pass; `list` prints 3 tiers, every line `[ok]` (Tier A: 11, B: 9, C: 3; 23 total).

- [ ] **Step 5: Commit**

```bash
git add rust/harness/src/
git commit -m "feat(m0): baseline distillation + capture/list subcommands

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 6: Baseline verify (`verify` subcommand)

**Files:**
- Modify: `rust/harness/src/baseline.rs` (add `compare`)
- Modify: `rust/harness/src/main.rs` (implement `Verify`)

- [ ] **Step 1: Write the failing tests for `compare`**

Append to `baseline.rs` (types + tests; implementation in Step 3):

```rust
/// A single gate failure. Wall time and SES hash are deliberately absent —
/// time is a trend metric and SES is compared semantically from M1.
#[derive(Debug, PartialEq)]
pub enum GateFailure {
    FinalState { expected: String, actual: String },
    ExitCode { expected: Option<i32>, actual: Option<i32> },
    IncompleteCount { expected: Option<i64>, actual: Option<i64> },
    ClearanceViolations { expected: Option<i64>, actual: Option<i64> },
    NormalizedScore { expected: f64, actual: f64 },
}

/// Compares a fresh run against a committed baseline. Counts/state/exit are
/// exact; normalized_score uses a relative epsilon (same engine, same seed
/// should be bit-identical, but JSON round-trips make a tiny epsilon the
/// honest bound).
pub fn compare(expected: &BaselineRecord, actual: &BaselineRecord) -> Vec<GateFailure> {
    let mut failures = Vec::new();
    if expected.final_state != actual.final_state {
        failures.push(GateFailure::FinalState {
            expected: expected.final_state.clone(),
            actual: actual.final_state.clone(),
        });
    }
    if expected.exit_code != actual.exit_code {
        failures.push(GateFailure::ExitCode {
            expected: expected.exit_code,
            actual: actual.exit_code,
        });
    }
    if expected.incomplete_count != actual.incomplete_count {
        failures.push(GateFailure::IncompleteCount {
            expected: expected.incomplete_count,
            actual: actual.incomplete_count,
        });
    }
    if expected.clearance_violations_total != actual.clearance_violations_total {
        failures.push(GateFailure::ClearanceViolations {
            expected: expected.clearance_violations_total,
            actual: actual.clearance_violations_total,
        });
    }
    if let (Some(e), Some(a)) = (expected.normalized_score, actual.normalized_score) {
        let epsilon = 1e-6 * e.abs().max(1.0);
        let diff = (e - a).abs();
        if diff > epsilon {
            failures.push(GateFailure::NormalizedScore { expected: e, actual: a });
        }
    } else if expected.normalized_score.is_some() != actual.normalized_score.is_some() {
        failures.push(GateFailure::NormalizedScore {
            expected: expected.normalized_score.unwrap_or(0.0),
            actual: actual.normalized_score.unwrap_or(0.0),
        });
    }
    failures
}

#[cfg(test)]
mod compare_tests {
    use super::*;

    fn record(state: &str, incomplete: Option<i64>, violations: Option<i64>, score: Option<f64>) -> BaselineRecord {
        BaselineRecord {
            schema_version: 1,
            engine: "java".into(),
            app_version: Some("2.5.0".into()),
            git_sha: Some("e7f9bdf1".into()),
            fixture: "x.dsn".into(),
            fixture_sha256: None,
            final_state: state.into(),
            exit_code: Some(0),
            incomplete_count: incomplete,
            maximum_count: Some(50),
            clearance_violations_total: violations,
            clearance_router_introduced: violations,
            normalized_score: score,
            optimizer_score: None,
            ses_sha256: None,
            autorouter_seconds: Some(10.0),
            optimizer_seconds: None,
            passes_completed: Some(20),
            captured_at_unix: 0,
            notes: None,
        }
    }

    #[test]
    fn identical_records_pass_with_zero_failures() {
        let a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        let b = record("COMPLETED", Some(0), Some(0), Some(990.0));
        assert_eq!(compare(&a, &b), Vec::new());
    }

    #[test]
    fn score_within_relative_epsilon_passes() {
        let a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        let b = record("COMPLETED", Some(0), Some(0), Some(990.0 + 1e-9));
        assert_eq!(compare(&a, &b), Vec::new());
    }

    #[test]
    fn count_and_state_regressions_fail() {
        let a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        let b = record("TIMED_OUT", Some(3), Some(1), Some(950.0));
        assert_eq!(compare(&a, &b).len(), 4);
    }

    #[test]
    fn wall_time_and_ses_hash_are_never_gated() {
        let mut a = record("COMPLETED", Some(0), Some(0), Some(990.0));
        a.autorouter_seconds = Some(10.0);
        a.ses_sha256 = Some("aaa".into());
        let mut b = record("COMPLETED", Some(0), Some(0), Some(990.0));
        b.autorouter_seconds = Some(999.0);
        b.ses_sha256 = Some("bbb".into());
        assert_eq!(compare(&a, &b), Vec::new());
    }
}
```

- [ ] **Step 2: Run to verify the new tests fail appropriately**

```bash
cd rust && cargo test -p epic-harness compare
```

Expected: compile error (`compare` not yet defined) — then implement in Step 3 and re-run. Since the tests and implementation are shown fully above, a single write then `cargo test -p epic-harness` should end green (4 compare tests + earlier tests).

- [ ] **Step 3: Implement `Verify` in `main.rs`**

Add `use baseline::BaselineRecord;` to the top of `main.rs` (next to the existing `use oracle::OracleRun;`), then replace the `Command::Verify { .. }` arm with:

```rust
        Command::Verify { tier } => {
            // Validate the tier name before paying for the gradle build.
            let selected = select_tiers(&tier_file, tier.as_deref())?;
            oracle::build_jar(&repo_root)?;
            let harness_root = tiers_path
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            let baselines_root = harness_root.join("baselines").join("java");
            let run_root = harness_root.join("runs").join("verify");
            let fixtures_rel = &tier_file.fixtures_root;
            let mut total_failures = 0usize;
            for tier in selected {
                for fixture in &tier.fixtures {
                    let baseline_path = baselines_root
                        .join(&tier.name)
                        .join(format!("{}.baseline.json", fixture.path));
                    if !baseline_path.is_file() {
                        bail!("missing baseline for {} — run capture first", fixture.path);
                    }
                    let expected: BaselineRecord = serde_json::from_str(
                        &std::fs::read_to_string(&baseline_path)?,
                    )
                    .with_context(|| format!("parsing {}", baseline_path.display()))?;

                    let dsn = fixtures_root.join(&fixture.path);
                    let work_dir = run_root.join(&tier.name).join(&fixture.path);
                    println!("[tier {}] verifying {}…", tier.name, fixture.path);
                    let run = oracle::run_oracle(
                        &java,
                        &jar,
                        &dsn,
                        &work_dir,
                        Duration::from_secs(fixture.timeout_seconds),
                        &cli.jvm_xmx,
                    )?;
                    let actual = baseline::distill(
                        &fixtures_rel.join(&fixture.path).to_string_lossy(),
                        &run,
                    );
                    let failures = baseline::compare(&expected, &actual);
                    if failures.is_empty() {
                        println!(
                            "  PASS ({:.1}s vs baseline {:.1}s)",
                            run.wall_seconds,
                            expected.autorouter_seconds.unwrap_or(0.0)
                                + expected.optimizer_seconds.unwrap_or(0.0)
                        );
                    } else {
                        total_failures += failures.len();
                        for f in &failures {
                            println!("  FAIL {f:?}");
                        }
                    }
                }
            }
            if total_failures == 0 {
                println!("verify: all gates passed");
                Ok(())
            } else {
                bail!("verify: {total_failures} gate failure(s)");
            }
        }
```

Note (from Task 5 quality review, folded forward): when loading committed baselines, check `record.schema_version == baseline::BASELINE_SCHEMA_VERSION` and bail with the file path otherwise; and add `#[serde(deny_unknown_fields)]` to `BaselineRecord` — unlike `manifest.rs` (which must tolerate Gson extras), the baseline format is written and read by this harness alone, so a strict reader makes format drift fail loudly instead of silently defaulting. The strict-reader/default-writer asymmetry is intentional.

Expected: all pass; clippy clean.

- [ ] **Step 5: Commit**

```bash
git add rust/harness/src/
git commit -m "feat(m0): verify subcommand — oracle stability gates on counts/state/score

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 7: Capture and commit Tier A/B/C golden baselines

**Files:**
- Create: `rust/harness/baselines/java/A/**/*.baseline.json` (11 files)
- Create: `rust/harness/baselines/java/B/**/*.baseline.json` (9 files)
- Create: `rust/harness/baselines/java/C/**/*.baseline.json` (3 files)
- Generated (not committed): `rust/harness/runs/**`

- [ ] **Step 1: Build the jar once, cleanly**

```bash
./gradlew -q executableJar && ls -la build/libs/freerouting-current-executable.jar
```

Expected: jar exists (~60-90 MB).

- [ ] **Step 2: Capture Tier A**

```bash
cd rust && EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- capture --tier A
```

Expected: 11 fixtures routed sequentially. Budgets sum to ~86 min but typical completion is far faster (bm08 ≈ 15-60 s; bm01 is the slowest). Each line prints state/incomplete/violations/score. Any `final_state` other than `COMPLETED` is recorded honestly — a Java failure is a valid baseline (v1.9 had 28 timeouts on the full corpus); do not retry to "get a better number", but DO note anything surprising in the commit message.

- [ ] **Step 3: Immediately verify Tier A (stability check)**

```bash
cd rust && EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- verify --tier A
```

Expected: `verify: all gates passed`. This proves the oracle is deterministic at the manifest level on this machine — the foundation every later parity claim rests on.

If any gate fails: stop. Investigate `runs/verify/…/stderr.log` for that fixture. Distinguish (a) true nondeterminism in the Java engine (document it in `.wolf/buglog.json` and the baseline `notes` field; widen the gate to a documented tolerance with a written justification), (b) machine noise/thermal throttling (rerun the single fixture), (c) harness bug (fix the harness). Never silently relax a gate.

- [ ] **Step 4: Capture + verify Tier B, then Tier C**

```bash
cd rust && EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- capture --tier B && EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- verify --tier B
cd rust && EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- capture --tier C && EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- verify --tier C
```

Expected: 9 then 3 fixtures (23 baselines total). Tier C's long pole is `bm04` (16 layers, 45-min budget; typically far faster). Capture timings are informational, so light parallel editing work is acceptable, but do not start another routing job concurrently.

- [ ] **Step 5: Confirm the baseline tree and no run artifacts leak into git**

```bash
find rust/harness/baselines -name '*.baseline.json' | wc -l   # expect 23
git status --short rust/ | head                                # only baselines/ staged-able
```

- [ ] **Step 6: Commit the baselines**

```bash
git add rust/harness/baselines/
git commit -m "feat(m0): golden baselines from Java oracle @ e7f9bdf1 (Tier A/B/C, 23 fixtures)

Oracle: build/libs/freerouting-current-executable.jar, default settings,
JDK 25. Gates proven stable by verify (counts/state/score identical across
independent runs).

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 8: CI workflow for the Rust workspace

**Files:**
- Create: `.github/workflows/rust-check.yml`

- [ ] **Step 1: Write the workflow**

```yaml
name: rust-check

on:
  push:
    branches: [master, main, epic/main]
    paths: ["rust/**", ".github/workflows/rust-check.yml"]
  pull_request:
    paths: ["rust/**", ".github/workflows/rust-check.yml"]

jobs:
  check:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: rust
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: rust
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```

Note: `cargo test` runs only unit tests here — all Java-dependent harness tests are `#[ignore]`d, so no JDK setup is needed on CI. (`rust-toolchain.toml` pins 1.93.0; dtolnay/rust-toolchain defers to it when the action's `toolchain` input is `stable`… actually it does NOT — the action input wins. The workflow intentionally uses `stable` on CI to catch future-MSRV breaks early, while local builds stay pinned. If pinning CI matters later, add `rust-version` enforcement then — do not add it now.)

- [ ] **Step 2: Validate the workflow file locally**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/rust-check.yml')); print('yaml ok')"
cd rust && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Expected: `yaml ok`, all green locally (the exact commands CI runs).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/rust-check.yml
git commit -m "ci(m0): fmt+clippy+test workflow for rust/ paths

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Task 9: Documentation + M0 exit-criteria validation

**Files:**
- Create: `rust/README.md`
- Modify: `CLAUDE.md` (add Rust section)
- Modify: `.wolf/memory.md` (session log line)

- [ ] **Step 1: Write `rust/README.md`**

```markdown
# EpicRouter Rust Workspace

Rust rewrite of the Freerouting autorouter (design:
`docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md`).
The Java tree under `src/` stays untouched as the parity oracle.

## Layout

- `crates/epic-geometry` — exact 45° integer geometry kernel (M1)
- `crates/epic-dsn` — Specctra DSN/SES I/O (M1)
- `crates/epic-board` — arena board model (M2)
- `crates/epic-index` — incremental spatial index (M2)
- `crates/epic-drc` — exact incremental DRC (M2/M6)
- `crates/epic-router` — staged router pipeline (M3+)
- `crates/epic-engine` — jobs, layered settings, seeds (M3)
- `crates/epic-cli` — `epicrouter` CLI (M3)
- `crates/epic-gui` — egui frontend (M9)
- `harness` — parity + benchmark harness against the Java oracle

## Commands

```bash
cd rust
cargo test --workspace              # unit tests (no Java needed)
cargo test -p epic-harness -- --ignored   # harness e2e tests (needs jar + JDK 25)

# Golden baselines (needs the oracle jar; see below)
./gradlew -q executableJar                      # from repo root
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- list
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- capture --tier A
EPIC_SKIP_GRADLE=1 cargo run -p epic-harness --release -- verify  --tier A
```

## Oracle prerequisites

- Jar: `build/libs/freerouting-current-executable.jar` via `./gradlew executableJar`.
- **Java 25 required** (system `java` may be 21). The harness auto-detects
  `~/.jdks/jdk-25*/bin/java`; override with `EPIC_ORACLE_JAVA`.
- `EPIC_SKIP_GRADLE=1` skips the jar build (use when iterating).
- The oracle jar is built from THIS tree at baseline `e7f9bdf1`; never modify
  Java sources without re-baselining and recording why.

## Baselines

`harness/baselines/java/<tier>/<fixture>.baseline.json` are committed
artifacts. `capture` regenerates them; `verify` re-runs the oracle and gates
on final_state, exit_code, incomplete_count, clearance_violations_total, and
normalized_score (relative epsilon 1e-6). Wall time is a trend, never a gate.
```

- [ ] **Step 2: Append a Rust section to `CLAUDE.md`**

Add after the environment section:

```markdown
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
```

- [ ] **Step 3: Run the full M0 exit-criteria check**

Per design §6 M0 row — run all three and paste results into the session log:

```bash
cd rust && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
./gradlew test
find rust/harness/baselines -name '*.baseline.json' | wc -l
```

Expected: all cargo/gradle green (gradle ~6 min, 558 tests — rerun solo if a timing-sensitive fixture fails under load); 23 baseline files.

- [ ] **Step 4: Update OpenWolf session memory**

Append to `.wolf/memory.md`:

```
| HH:MM | M0 complete: rust/ workspace (9 crates + epic-harness), Tier A/B/C golden baselines (23 fixtures) captured+verified, CI workflow | rust/**, .github/workflows/rust-check.yml, CLAUDE.md | committed | ~NN |
```

- [ ] **Step 5: Final commit**

```bash
git add rust/README.md CLAUDE.md .wolf/memory.md
git commit -m "docs(m0): rust workspace README, CLAUDE.md guidance, session log

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## Self-review notes (already applied)

1. **Spec coverage vs design §6 M0 row** — "Rust workspace scaffold" = Task 1; "harness skeleton" = Tasks 2-6; "golden baselines from Java" = Task 7; exit criteria (Tier A/B/C manifests committed; `cargo test` + `./gradlew test` green) = Tasks 7 & 9. The §5 harness gates that are *implementable* in M0 (determinism spot-check via `verify`, versioned/regenerable baselines via `capture`) are covered; router/index/geometry parity gates arrive with their milestones.
2. **Placeholder scan** — no TBD/TODO/vague steps; every code step shows complete code (the sha256 test asserts stability + format rather than a hard-coded digest constant).
3. **Type consistency** — `TierFile`/`Tier`/`FixtureRef` (Task 2), `RoutingResultManifest` mirror (Task 3), `OracleRun`/`oracle_argv`/`run_oracle`/`find_repo_root`/`resolve_java`/`jar_path`/`build_jar` (Task 4), `BaselineRecord`/`distill`/`compare`/`GateFailure`/`sha256_file` (Tasks 5-6) are used with those exact names across `main.rs`; signatures match at every call site shown.
4. **Known honest simplifications** (fine for M0, revisit at M3): `resolve_tiers_path` guesses relative-path roots for convenience invocation from any directory; `BaselineRecord.fixture` is repo-relative (per the Task 5 amendment — self-describing against the repo root, the tier is still carried by the `baselines/java/<tier>/` directory); wall-time figures are informational only.
```
