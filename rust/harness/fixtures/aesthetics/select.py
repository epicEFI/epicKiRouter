#!/usr/bin/env python3
"""The M8-T1 committed 21-board PCBench sample: selection + golden capture.

The rule (the plan's Key facts, materialized verbatim):

  1. Population: every PCBench directory under
     ``scripts/benchmark/fixtures/PCBench/`` carrying BOTH ``unrouted.dsn``
     and ``reference-routed.dsn`` (the 1,157 paired dirs).
  2. EXCLUDE the three PCBench dirs already in ``harness/config/tiers.yaml``
     (1-Wire-Wing-pcb_1-Wire_Wing, 1Bitsy_1bitsy,
     front-end-modules_LimeSDR_Sony) — they are parity instruments, kept
     separate → 1,154 eligible.
  3. Stratify by ``unrouted.dsn`` BYTE SIZE terciles: sort eligible dirs
     ascending by (unrouted.dsn byte size, dirname); with n = 1154 the
     tercile split is ceil(n/3)=385 / 385 / 384 (tercile 0 = ranks
     1-385, tercile 1 = ranks 386-770, tercile 2 = ranks 771-1154).
  4. Per tercile: the 7 lowest sha256 of the DIRECTORY NAME string
     (hex digest, lexicographic; tie-break dirname ascending) → 21 boards.

Re-runnable: `PYTHONSAFEPATH=1 python3 rust/harness/fixtures/aesthetics/select.py
[--goldens-only]` from the repository root (PYTHONSAFEPATH=1 is REQUIRED:
the file is named `select.py`, which shadows the stdlib `select` module
when the script directory is on sys.path — the default run dies in
`import subprocess`) (java-free; the golden capture spawns the
built ``epic-harness`` binary — run ``cargo build -p epic-harness``
first). The selection is a pure function of the tree: the same tree
SHA always yields the same 21 names.
"""

from __future__ import annotations

import argparse
import hashlib
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[4]
PCBENCH = REPO_ROOT / "scripts/benchmark/fixtures/PCBench"
TIER_CONTEXT = [
    "1-Wire-Wing-pcb_1-Wire_Wing",
    "1Bitsy_1bitsy",
    "front-end-modules_LimeSDR_Sony",
]
OUT_DIR = REPO_ROOT / "rust/harness/fixtures/aesthetics"
GOLDEN_DIR = OUT_DIR / "golden"
GENERATED_AT_TREE_SHA = "ccd1e6024"
PER_TERCILE = 7

# The tercile boundaries at n=1154 (the eligible population this
# sample was cut from; re-derived on every run and asserted).
EXPECTED_N = 1154
BOUNDARY_1 = 385
BOUNDARY_2 = 770


def paired_dirs() -> list[Path]:
    """Every PCBench dir carrying BOTH unrouted.dsn + reference-routed.dsn."""
    result = []
    for child in sorted(PCBENCH.iterdir()):
        if not child.is_dir():
            continue
        if (child / "unrouted.dsn").is_file() and (child / "reference-routed.dsn").is_file():
            result.append(child)
    return result


def select_sample() -> tuple[list[dict], list[dict], list[dict]]:
    """Returns (the 21 sample rows, the 3 tier-context rows, tercile bounds)."""
    dirs = paired_dirs()
    eligible = [d for d in dirs if d.name not in TIER_CONTEXT]
    n = len(eligible)
    if n != EXPECTED_N:
        print(
            f"FATAL: eligible population is {n}, expected {EXPECTED_N} — "
            "the PCBench tree changed; the committed sample is a function "
            "of tree ccd1e6024 and must be re-cut through a new charter, "
            "not a silent re-selection.",
            file=sys.stderr,
            )
        raise SystemExit(2)
    # Sort ascending by (unrouted.dsn byte size, dirname); stable, total.
    eligible.sort(key=lambda d: ((d / "unrouted.dsn").stat().st_size, d.name))
    terciles: list[list[Path]] = [
        eligible[:BOUNDARY_1],
        eligible[BOUNDARY_1:BOUNDARY_2],
        eligible[BOUNDARY_2:],
    ]
    sample = []
    tercile_bounds = []
    for t_index, tercile in enumerate(terciles):
        rows = sorted(
            tercile,
            key=lambda d: (hashlib.sha256(d.name.encode()).hexdigest(), d.name),
        )[:PER_TERCILE]
        size_lo = (tercile[0] / "unrouted.dsn").stat().st_size
        size_hi = (tercile[-1] / "unrouted.dsn").stat().st_size
        tercile_bounds.append((t_index, len(tercile), size_lo, size_hi))
        for row in rows:
            sample.append(
                {
                    "dirname": row.name,
                    "tercile": t_index,
                    "unrouted_bytes": (row / "unrouted.dsn").stat().st_size,
                    "sha256_dirname": hashlib.sha256(row.name.encode()).hexdigest(),
                }
            )
    tier_rows = []
    for name in TIER_CONTEXT:
        p = PCBENCH / name
        tier_rows.append(
            {
                "dirname": name,
                "unrouted_bytes": (p / "unrouted.dsn").stat().st_size,
                "sha256_dirname": hashlib.sha256(name.encode()).hexdigest(),
            }
        )
    tier_rows.sort(key=lambda r: r["dirname"])
    return sample, tier_rows, tercile_bounds


def write_sample_yaml(sample: list[dict], tier_rows: list[dict], tercile_bounds) -> None:
    lines = [
        "# The M8-T1 committed 21-board PCBench sample (docs/superpowers/plans/"
        "2026-09-28-epicrouter-m8-gloss.md, Task 1).",
        "#",
        "# Selection rule (materialized verbatim):",
        "#   1. Population: the PCBench dirs carrying BOTH unrouted.dsn and",
        "#      reference-routed.dsn (1,157 at the generated-at tree).",
        "#   2. EXCLUDED: the three PCBench dirs in harness/config/tiers.yaml",
        "#      (1-Wire-Wing-pcb_1-Wire_Wing, 1Bitsy_1bitsy,",
        "#      front-end-modules_LimeSDR_Sony) — parity instruments, kept",
        "#      separate → 1,154 eligible.",
        "#   3. Stratify by unrouted.dsn BYTE SIZE terciles (ascending by",
        "#      (unrouted.dsn bytes, dirname); n=1154 → 385/385/384).",
        "#   4. Per tercile: the 7 lowest sha256(dirname) hex digests",
        "#      (lexicographic; tie-break dirname ascending) → 21 boards.",
        "#",
        f"# Generated at tree SHA {GENERATED_AT_TREE_SHA} (branch epic/main).",
        "# Re-cut through: PYTHONSAFEPATH=1 python3 rust/harness/fixtures/aesthetics/select.py",
        "# (a pure function of the tree — the same tree always yields the",
        "# same 21 names; the eligible-population count is asserted).",
        "#",
        f"# Tercile boundaries at capture: t0 = ranks 1-{BOUNDARY_1} "
        f"(unrouted.dsn {tercile_bounds[0][2]}-{tercile_bounds[0][3]} bytes),",
        f"# t1 = ranks {BOUNDARY_1 + 1}-{BOUNDARY_2} "
        f"({tercile_bounds[1][2]}-{tercile_bounds[1][3]} bytes), "
        f"t2 = ranks {BOUNDARY_2 + 1}-{EXPECTED_N} "
        f"({tercile_bounds[2][2]}-{tercile_bounds[2][3]} bytes).",
        "",
        "sample:",
    ]
    for row in sample:
        lines.append(
            f"  - dirname: {row['dirname']}"
        )
        lines.append(f"    tercile: {row['tercile']}")
        lines.append(f"    unrouted_bytes: {row['unrouted_bytes']}")
        lines.append(f"    sha256_dirname: {row['sha256_dirname']}")
    lines.append("")
    lines.append("# The three tier-context boards (excluded from the sample; their")
    lines.append("# reference goldens ship too — the parity instruments' context rows).")
    lines.append("tier_context:")
    for row in tier_rows:
        lines.append(f"  - dirname: {row['dirname']}")
        lines.append(f"    unrouted_bytes: {row['unrouted_bytes']}")
        lines.append(f"    sha256_dirname: {row['sha256_dirname']}")
    lines.append("")
    out = OUT_DIR / "sample.yaml"
    out.write_text("\n".join(lines))
    print(f"wrote {out} ({len(sample)} sample + {len(tier_rows)} tier-context rows)")


def capture_goldens(harness: str) -> None:
    """Produces every golden via the reference door (java-free)."""
    GOLDEN_DIR.mkdir(parents=True, exist_ok=True)
    sample, tier_rows, _ = select_sample()
    names = [row["dirname"] for row in sample] + [row["dirname"] for row in tier_rows]
    for name in names:
        dsn = PCBENCH / name / "reference-routed.dsn"
        proc = subprocess.run(
            [harness, "aesthetics", "--dsn", str(dsn)],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            print(f"FATAL: {name}: harness exit {proc.returncode}: {proc.stderr}", file=sys.stderr)
            raise SystemExit(3)
        json_bytes = proc.stdout
        # Normalize to exactly what the face printed + trailing newline.
        if not json_bytes.endswith("\n"):
            json_bytes += "\n"
        out = GOLDEN_DIR / f"{name}.json"
        out.write_text(json_bytes)
        print(f"golden {name}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--goldens-only",
        action="store_true",
        help="skip sample.yaml regeneration; re-capture the goldens only",
    )
    parser.add_argument(
        "--harness",
        default=str(REPO_ROOT / "rust/target/debug/epic-harness"),
        help="the epic-harness binary for the golden capture",
    )
    args = parser.parse_args()
    if not args.goldens_only:
        sample, tier_rows, tercile_bounds = select_sample()
        write_sample_yaml(sample, tier_rows, tercile_bounds)
    capture_goldens(args.harness)


if __name__ == "__main__":
    main()
