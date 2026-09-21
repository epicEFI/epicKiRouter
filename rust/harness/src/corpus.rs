//! Differential geometry corpus (M1a Task 9): seeded case generation,
//! Java-oracle golden capture, and epic-geometry comparison.
//!
//! Encoding contract (mirrors `rust/harness/oracle/GeometryCorpusOracle.java`):
//! - int point `[x, y]`; float point `[x_bits, y_bits]` (raw f64 bit patterns);
//! - octagon `[lx, ly, rx, uy, ulx, lrx, llx, urx]` (IntOctagon ctor order);
//!   box `[llx, lly, urx, ury]`; direction/vector `[x, y]`;
//! - line `{"a": {"x", "y"}, "b": {"x", "y"}}`; float line same with bit arrays;
//! - doubles always as `{"bits": i64}` (`f64::to_bits`), floats as `{"fbits": i32}`;
//! - `Line::intersection` parallel/collinear: Java yields an infinite
//!   RationalPoint (z = 0), Rust yields `None` — BOTH serialize as
//!   `{"kind":"Infinity"}`; `FloatLine::intersection` yields Java null / Rust
//!   `None` — both serialize as `{"kind":"Null"}`;
//! - Side -> "left"/"collinear"/"right", Signum -> "pos"/"zero"/"neg";
//!   compareTo-style results as `{"cmp": signum}`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};

use epic_geometry::direction::Direction;
use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::limits::CRIT_INT;
use epic_geometry::line::Line;
use epic_geometry::line_segment::LineSegment;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::side::Side;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;
use epic_geometry::vector::Vector;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum CorpusCommand {
    /// Generate the seeded geometry case corpus (deterministic per seed).
    Generate {
        /// RNG seed; two runs with the same seed are byte-identical.
        #[arg(long, default_value_t = 20260912)]
        seed: u64,
        /// Number of cases to generate.
        #[arg(long, default_value_t = 5000)]
        cases: usize,
        /// Output path for the case file (relative to cwd).
        #[arg(long, default_value = "harness/corpus/geometry-cases.jsonl")]
        out: PathBuf,
    },
    /// Evaluate every case with the Java oracle jar and write the goldens.
    Golden {
        #[arg(long, default_value = "harness/corpus/geometry-cases.jsonl")]
        cases: PathBuf,
        #[arg(long, default_value = "harness/corpus/geometry-golden.jsonl")]
        out: PathBuf,
    },
    /// Evaluate every case with epic-geometry and diff against the goldens.
    Compare {
        #[arg(long, default_value = "harness/corpus/geometry-cases.jsonl")]
        cases: PathBuf,
        #[arg(long, default_value = "harness/corpus/geometry-golden.jsonl")]
        golden: PathBuf,
    },
}

pub fn run(cmd: CorpusCommand) -> Result<()> {
    match cmd {
        CorpusCommand::Generate { seed, cases, out } => generate(seed, cases, &out),
        CorpusCommand::Golden { cases, out } => golden(&cases, &out),
        CorpusCommand::Compare { cases, golden } => compare(&cases, &golden),
    }
}

/// Resolves an input path relative to cwd, falling back to repo-root-relative
/// (so the corpus commands work from both `rust/` and the repo root).
fn resolve_input(given: &Path) -> PathBuf {
    if given.is_file() || given.is_absolute() {
        given.to_path_buf()
    } else {
        match crate::oracle::find_repo_root() {
            Ok(root) if root.join(given).is_file() => root.join(given),
            _ => given.to_path_buf(),
        }
    }
}

// ---------------------------------------------------------------------------
// Seeded RNG (local splitmix64 — no deps, no epic-geometry internals)
// ---------------------------------------------------------------------------

struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn i32_in(&mut self, lo: i32, hi: i32) -> i32 {
        // Range computed in i64: lo=-(1<<30), hi=1<<30 spans 2^31 values and
        // would overflow an i32 subtraction.
        lo + self.below(((hi as i64) - (lo as i64) + 1) as u64) as i32
    }
}

/// The 8 canonical 45-degree unit vectors, indexed so that (i, i+1, i+2, i+3)
/// mod 8 are pairwise non-parallel.
const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// Magnitude distribution: ~70% board-realistic +-10^4, ~25% up to +-CRIT_INT,
/// ~5% beyond (up to +-2^30) to exercise wraparound paths.
fn rand_coord(rng: &mut SplitMix64) -> i32 {
    match rng.below(100) {
        0..=69 => rng.i32_in(-10_000, 10_000),
        70..=94 => rng.i32_in(-CRIT_INT, CRIT_INT),
        _ => rng.i32_in(-(1 << 30), 1 << 30),
    }
}

fn rand_pt(rng: &mut SplitMix64) -> Value {
    json!([rand_coord(rng), rand_coord(rng)])
}

/// A proper (non-degenerate) 45-degree line: anchor + canonical direction.
fn rand_line(rng: &mut SplitMix64) -> Value {
    line_val(
        rand_coord(rng),
        rand_coord(rng),
        rng.below(8) as usize,
        rng.i32_in(1, 4000),
    )
}

fn line_val(ax: i32, ay: i32, dir_idx: usize, k: i32) -> Value {
    let (dx, dy) = DIRS[dir_idx % 8];
    json!({
        "a": {"x": ax, "y": ay},
        "b": {"x": ax + dx * k, "y": ay + dy * k}
    })
}

fn rand_dir(rng: &mut SplitMix64) -> Value {
    let (dx, dy) = DIRS[rng.below(8) as usize];
    let k = rng.i32_in(1, 2000);
    json!([dx * k, dy * k])
}

/// Plausible octagon args: axis bounds define a rectangle, the four diagonal
/// intercepts are sampled inside it; normalize() repairs inconsistencies.
fn rand_oct(rng: &mut SplitMix64, sliver: bool) -> Value {
    let lx = rng.i32_in(-3000, 3000);
    let ly = rng.i32_in(-3000, 3000);
    let w = if sliver { 1 } else { rng.i32_in(1, 2000) };
    let h = if sliver {
        rng.i32_in(1, 2)
    } else {
        rng.i32_in(1, 2000)
    };
    let rx = lx + w;
    let uy = ly + h;
    json!([
        lx,
        ly,
        rx,
        uy,
        lx + rng.i32_in(0, w),
        rx - rng.i32_in(0, w),
        lx + rng.i32_in(0, w),
        rx - rng.i32_in(0, w)
    ])
}

fn rand_box(rng: &mut SplitMix64) -> Value {
    let lx = rng.i32_in(-3000, 3000);
    let ly = rng.i32_in(-3000, 3000);
    json!([lx, ly, lx + rng.i32_in(0, 2000), ly + rng.i32_in(0, 2000)])
}

/// Float coordinates: integral or half-valued doubles (exactly representable).
fn rand_fpt(rng: &mut SplitMix64) -> Value {
    let base = rng.i32_in(-10_000, 10_000) as f64;
    let x = if rng.below(4) == 0 { base + 0.5 } else { base };
    let base_y = rng.i32_in(-10_000, 10_000) as f64;
    let y = if rng.below(4) == 0 {
        base_y + 0.5
    } else {
        base_y
    };
    json!([x.to_bits() as i64, y.to_bits() as i64])
}

fn rand_fline(rng: &mut SplitMix64) -> Value {
    json!({"a": rand_fpt(rng), "b": rand_fpt(rng)})
}

/// N lines with pairwise non-parallel directions (index stride != 4 mod 8),
/// the shape the LineSegment/Simplex/Polyline constructors expect.
fn rand_line_fan(rng: &mut SplitMix64, n: usize, forty_five: bool) -> Value {
    let start = rng.below(8) as usize;
    let strides = [0usize, 1, 3, 2];
    let lines: Vec<Value> = (0..n)
        .map(|i| {
            if forty_five {
                line_val(
                    rand_coord(rng),
                    rand_coord(rng),
                    start + strides[i % strides.len()],
                    rng.i32_in(1, 4000),
                )
            } else {
                let ax = rand_coord(rng);
                let ay = rand_coord(rng);
                json!({
                    "a": {"x": ax, "y": ay},
                    "b": {"x": rand_coord(rng), "y": rand_coord(rng)}
                })
            }
        })
        .collect();
    json!(lines)
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// (id code, ops, weight) per family. Task 10 weighting: the router hot
/// path (octagons + lines + boxes + segments) carries ~65% of the cases;
/// every family is kept so the tail ops stay differentially covered.
const FAMILIES: &[(&str, &[&str], u32)] = &[
    (
        "ipt",
        &[
            "intpoint.determinant",
            "intpoint.difference_by",
            "intpoint.fortyfive_degree_projection",
            "intpoint.perpendicular_projection",
            "intpoint.surrounding_octagon",
        ],
        3,
    ),
    (
        "vec",
        &[
            "vector.side_of",
            "vector.projection",
            "vector.turn_90_degree",
        ],
        2,
    ),
    (
        "dir",
        &[
            "direction.get_instance",
            "direction.turn_45_degree",
            "direction.compare_to",
        ],
        2,
    ),
    ("pnt", &["point.compare_xy", "point.translate_by"], 1),
    (
        "box",
        &[
            "intbox.intersection",
            "intbox.union",
            "intbox.offset_double",
            "intbox.area",
            "intbox.circumference",
            "intbox.intersects",
            "intbox.cutout",
        ],
        6,
    ),
    (
        "oct",
        &[
            "octagon.normalize",
            "octagon.intersection",
            "octagon.union",
            "octagon.offset",
            "octagon.intersects",
            "octagon.overlaps",
            "octagon.contains_point",
            "octagon.contains_float_point",
            "octagon.area",
            "octagon.bounding_box",
            "octagon.corner",
            "octagon.enlarge",
            "octagon.compare_edge",
            "octagon.side_of_border_line",
        ],
        13,
    ),
    (
        "lin",
        &[
            "line.get_instance",
            "line.intersection",
            "line.intersection_approx",
            "line.side_of_point",
            "line.side_of_intersection",
            "line.perpendicular_projection",
            "line.fast_equals",
            "line.compare_to",
            "line.direction",
            "line.length",
        ],
        11,
    ),
    (
        "seg",
        &[
            "line_segment.intersection",
            "line_segment.bounding_box",
            "line_segment.sort_endpoints_in_xy",
        ],
        3,
    ),
    (
        "tsh",
        &[
            "tileshape.from_8_ints",
            "tileshape.intersection_box_oct",
            "tileshape.from_points",
        ],
        2,
    ),
    // No `simplex.remove_redundant` op: Java `removeRedundantLines()` is
    // package-private (Simplex.java:884); pruning is covered transitively by
    // `Simplex.getInstance` (:46) and `intersection` (:629). Task 10 adds
    // deliberately-redundant `simplex.get_instance` cases to exercise it.
    ("spx", &["simplex.get_instance", "simplex.intersection"], 3),
    (
        "ply",
        &[
            "polyline.ctor",
            "polyline.bounding_box",
            "polyline.length_approx",
            "polyline.offset_shape",
            "polyline.is_multiple_of_45_degree",
        ],
        2,
    ),
    (
        "cir",
        &["circle.intersects_octagon", "circle.bounding_octagon"],
        1,
    ),
    (
        "fpt",
        &[
            "floatpoint.round",
            "floatpoint.round_to_grid",
            "floatpoint.distance_square",
            "floatpoint.inside_circle",
        ],
        1,
    ),
    (
        "fln",
        &["floatline.intersection", "floatline.projection"],
        1,
    ),
];

fn build_args(rng: &mut SplitMix64, op: &str) -> Value {
    match op {
        "intpoint.determinant"
        | "intpoint.difference_by"
        | "intpoint.fortyfive_degree_projection"
        | "point.compare_xy" => {
            json!({"a": rand_pt(rng), "b": rand_pt(rng)})
        }
        "intpoint.perpendicular_projection" => {
            json!({"p": rand_pt(rng), "line": rand_line(rng)})
        }
        "intpoint.surrounding_octagon" => json!({"a": rand_pt(rng)}),
        "vector.side_of" | "vector.projection" => {
            json!({"a": rand_pt(rng), "b": rand_pt(rng)})
        }
        "vector.turn_90_degree" => {
            json!({"a": rand_pt(rng), "factor": rng.i32_in(-2, 2)})
        }
        "direction.get_instance" => json!({"v": rand_dir(rng)}),
        "direction.turn_45_degree" => {
            json!({"v": rand_dir(rng), "factor": rng.i32_in(-4, 4)})
        }
        "direction.compare_to" => json!({"a": rand_dir(rng), "b": rand_dir(rng)}),
        "point.translate_by" => json!({"a": rand_pt(rng), "v": rand_pt(rng)}),
        "intbox.intersection" | "intbox.union" | "intbox.intersects" => {
            json!({"a": rand_box(rng), "b": rand_box(rng)})
        }
        "intbox.offset_double" => {
            let bits = (rng.i32_in(-500, 500) as f64).to_bits() as i64;
            json!({"a": rand_box(rng), "dist_bits": bits})
        }
        "intbox.area" | "intbox.circumference" => json!({"a": rand_box(rng)}),
        "intbox.cutout" => json!({"a": rand_box(rng), "b": rand_box(rng)}),
        "octagon.normalize" => {
            let sliver = rng.below(8) == 0;
            json!({"a": rand_oct(rng, sliver)})
        }
        "octagon.intersection"
        | "octagon.union"
        | "octagon.intersects"
        | "octagon.overlaps"
        | "octagon.compare_edge" => {
            let mut args = json!({"a": rand_oct(rng, false), "b": rand_oct(rng, false)});
            if op == "octagon.compare_edge" {
                args["edge"] = json!(rng.below(8) as i32);
            }
            args
        }
        "octagon.offset" | "octagon.enlarge" => {
            let key = if op == "octagon.offset" {
                "dist_bits"
            } else {
                "offset_bits"
            };
            let bits = (rng.i32_in(-500, 500) as f64).to_bits() as i64;
            json!({"a": rand_oct(rng, false), key: bits})
        }
        "octagon.contains_point" => json!({"a": rand_oct(rng, false), "p": rand_pt(rng)}),
        "octagon.contains_float_point" => json!({"a": rand_oct(rng, false), "p": rand_fpt(rng)}),
        "octagon.area" | "octagon.bounding_box" => json!({"a": rand_oct(rng, false)}),
        "octagon.corner" => json!({"a": rand_oct(rng, false), "no": rng.below(8) as i32}),
        "octagon.side_of_border_line" => {
            json!({
                "a": rand_oct(rng, false),
                "x": rng.i32_in(-4000, 4000),
                "y": rng.i32_in(-4000, 4000),
                "no": rng.below(8) as i32
            })
        }
        "line.get_instance" => json!({"a": rand_pt(rng), "dir": rand_dir(rng)}),
        "line.intersection"
        | "line.intersection_approx"
        | "line.fast_equals"
        | "line.compare_to" => {
            json!({"a": rand_line(rng), "b": rand_line(rng)})
        }
        "line.side_of_point" | "line.perpendicular_projection" => {
            json!({"a": rand_line(rng), "p": rand_pt(rng)})
        }
        "line.side_of_intersection" => {
            json!({"a": rand_line(rng), "l1": rand_line(rng), "l2": rand_line(rng)})
        }
        "line.direction" | "line.length" => json!({"a": rand_line(rng)}),
        "line_segment.intersection" => {
            json!({
                "s": rand_line(rng), "m": rand_line(rng), "e": rand_line(rng),
                "s2": rand_line(rng), "m2": rand_line(rng), "e2": rand_line(rng)
            })
        }
        "line_segment.bounding_box" | "line_segment.sort_endpoints_in_xy" => {
            json!({"s": rand_line(rng), "m": rand_line(rng), "e": rand_line(rng)})
        }
        "tileshape.from_8_ints" => {
            let sliver = rng.below(6) == 0;
            json!({"v": rand_oct(rng, sliver)})
        }
        "tileshape.intersection_box_oct" => {
            json!({"box": rand_box(rng), "oct": rand_oct(rng, false)})
        }
        "tileshape.from_points" => {
            let n = 2 + rng.below(3) as usize;
            let pts: Vec<Value> = (0..n).map(|_| rand_pt(rng)).collect();
            json!({"points": pts})
        }
        "simplex.get_instance" | "simplex.intersection" => {
            let n = 3 + rng.below(2) as usize;
            let mut lines = match rand_line_fan(rng, n, true) {
                Value::Array(v) => v,
                _ => unreachable!("rand_line_fan emits an array"),
            };
            if op == "simplex.get_instance" && rng.below(2) == 0 {
                // Deliberately-redundant fan (Task 10): 1-2 extra lines
                // drawn from the full direction set collide with the fan,
                // producing compareTo == 0 duplicates and nested
                // half-planes for the stable sort + removeRedundantLines
                // pruning inside Simplex.getInstance.
                let extras = 1 + rng.below(2) as usize;
                for _ in 0..extras {
                    lines.push(line_val(
                        rand_coord(rng),
                        rand_coord(rng),
                        rng.below(8) as usize,
                        rng.i32_in(1, 4000),
                    ));
                }
            }
            let mut args = json!({ "lines": Value::Array(lines) });
            if op == "simplex.intersection" {
                args["lines2"] = rand_line_fan(rng, 3, true);
            }
            args
        }
        "polyline.ctor"
        | "polyline.bounding_box"
        | "polyline.length_approx"
        | "polyline.offset_shape"
        | "polyline.is_multiple_of_45_degree" => {
            let n = 2 + rng.below(3) as usize;
            let forty_five = rng.below(2) == 0;
            let mut args = json!({"lines": rand_line_fan(rng, n, forty_five)});
            if op == "polyline.offset_shape" {
                args["half_width"] = json!(rng.i32_in(1, 40));
                args["no"] = json!(0);
            }
            args
        }
        "circle.intersects_octagon" => {
            json!({"center": rand_pt(rng), "radius": rng.i32_in(0, 10_000), "oct": rand_oct(rng, false)})
        }
        "circle.bounding_octagon" => {
            json!({"center": rand_pt(rng), "radius": rng.i32_in(0, 10_000)})
        }
        "floatpoint.round" | "floatpoint.distance_square" => {
            let key = if op == "floatpoint.round" { "p" } else { "b" };
            let mut args = json!({"a": rand_fpt(rng)});
            args[key] = rand_fpt(rng);
            args
        }
        "floatpoint.round_to_grid" => {
            json!({"p": rand_fpt(rng), "h": rng.i32_in(1, 100), "v": rng.i32_in(1, 100)})
        }
        "floatpoint.inside_circle" => {
            json!({
                "p": rand_fpt(rng), "p1": rand_fpt(rng),
                "p2": rand_fpt(rng), "p3": rand_fpt(rng)
            })
        }
        "floatline.intersection" => json!({"a": rand_fline(rng), "b": rand_fline(rng)}),
        "floatline.projection" => json!({"a": rand_fline(rng), "p": rand_fpt(rng)}),
        _ => unreachable!("generator op table drift: {op}"),
    }
}

/// Deliberate edge cases, emitted every 9th case: parallel/collinear/identical
/// line pairs (the Infinity mapping), disjoint empty-producing shapes,
/// degenerate slivers, and coordinates at exactly +-CRIT_INT and +-46341.
fn edge_case(idx: usize) -> (&'static str, Value) {
    let crit = CRIT_INT;
    let f = |v: f64| v.to_bits() as i64;
    match idx % 30 {
        0 => (
            "line.intersection",
            json!({"a": line_val(0, 0, 0, 10), "b": line_val(0, 5, 0, 10)}),
        ),
        1 => (
            "line.intersection",
            json!({"a": line_val(0, 0, 1, 10), "b": line_val(5, 5, 1, 10)}),
        ),
        2 => (
            "line.intersection",
            json!({"a": line_val(3, 4, 2, 10), "b": line_val(3, 4, 2, 10)}),
        ),
        3 => (
            "line.intersection",
            json!({"a": line_val(0, 0, 0, 10), "b": line_val(5, -5, 2, 10)}),
        ),
        4 => (
            "intbox.intersection",
            json!({"a": [0, 0, 10, 10], "b": [20, 20, 30, 30]}),
        ),
        5 => (
            "octagon.intersection",
            json!({"a": [0, 0, 10, 10, 0, 10, 0, 10], "b": [100, 100, 110, 110, 100, 110, 100, 110]}),
        ),
        6 => (
            "octagon.intersection",
            json!({"a": [crit, crit, -crit, -crit, crit, -crit, crit, -crit], "b": [0, 0, 4, 4, 0, 4, 0, 4]}),
        ),
        7 => ("octagon.normalize", json!({"a": [0, 0, 1, 1, 0, 1, 0, 1]})),
        8 => ("octagon.normalize", json!({"a": [5, 5, 5, 9, 5, 5, 5, 9]})),
        9 => (
            "intbox.intersection",
            json!({"a": [-crit, -crit, crit, crit], "b": [0, 0, 1, 1]}),
        ),
        10 => (
            "octagon.intersection",
            json!({
                "a": [-crit, -crit, crit, crit, -crit, crit, -crit, crit],
                "b": [-3, -3, 12, 12, 0, 15, 0, 15]
            }),
        ),
        11 => (
            "intpoint.determinant",
            json!({"a": [46341, 0], "b": [0, 46341]}),
        ),
        12 => (
            "intpoint.determinant",
            json!({"a": [crit, 0], "b": [0, crit]}),
        ),
        13 => ("vector.side_of", json!({"a": [crit, 1], "b": [-crit, 1]})),
        14 => ("direction.get_instance", json!({"v": [1 << 30, 1 << 30]})),
        15 => (
            "line.intersection_approx",
            json!({
                "a": line_val(-(1 << 30), -(1 << 30), 1, 1 << 29),
                "b": line_val(-(1 << 30), 1 << 30, 7, 1 << 29)
            }),
        ),
        16 => (
            "simplex.get_instance",
            json!({"lines": [line_val(0, 0, 0, 10), line_val(1, 0, 0, 10), line_val(0, 5, 2, 10)]}),
        ),
        17 => (
            "polyline.offset_shape",
            json!({
                "lines": [line_val(0, 0, 1, 10), line_val(10, 10, 3, 10), line_val(0, 20, 5, 10)],
                "half_width": 3, "no": 0
            }),
        ),
        18 => (
            "polyline.offset_shape",
            json!({
                "lines": [
                    json!({"a": {"x": 0, "y": 0}, "b": {"x": 7, "y": 0}}),
                    json!({"a": {"x": 7, "y": 0}, "b": {"x": 7, "y": 11}})
                ],
                "half_width": 2, "no": 0
            }),
        ),
        19 => (
            "circle.bounding_octagon",
            json!({"center": [7, -9], "radius": 0}),
        ),
        20 => ("floatpoint.round", json!({"p": [f(-2.5), f(-0.5)]})),
        21 => (
            "floatline.intersection",
            json!({
                "a": {"a": [f(0.0), f(0.0)], "b": [f(4.0), f(0.0)]},
                "b": {"a": [f(0.0), f(2.5)], "b": [f(9.5), f(2.5)]}
            }),
        ),
        22 => (
            "octagon.contains_point",
            json!({"a": [0, 0, 10, 10, 0, 10, 0, 10], "p": [0, 10]}),
        ),
        23 => (
            "intbox.cutout",
            json!({"a": [0, 0, 10, 10], "b": [0, 0, 10, 10]}),
        ),
        24 => (
            "line.side_of_intersection",
            json!({
                "a": line_val(0, 0, 2, 10),
                "l1": line_val(0, 0, 0, 10),
                "l2": line_val(0, 3, 0, 10)
            }),
        ),
        25 => (
            "tileshape.from_points",
            json!({"points": [[3, 4], [3, 4], [9, 1]]}),
        ),
        // Negative-sign wraparound: (-46341)^2 overflows i32 exactly like
        // the positive case 11.
        26 => (
            "intpoint.determinant",
            json!({"a": [-46341, 0], "b": [0, -46341]}),
        ),
        27 => (
            "intpoint.determinant",
            json!({"a": [-crit, 0], "b": [0, crit]}),
        ),
        // Parallel closing/middle verticals: the start corner is an
        // infinite RationalPoint and sortEndpointsInXY decides on it (T22).
        28 => (
            "line_segment.sort_endpoints_in_xy",
            json!({
                "s": {"a": {"x": 0, "y": 1}, "b": {"x": 0, "y": 9}},
                "m": {"a": {"x": 3, "y": 5}, "b": {"x": 3, "y": -5}},
                "e": {"a": {"x": -4, "y": -2}, "b": {"x": 6, "y": -2}}
            }),
        ),
        29 => (
            "simplex.get_instance",
            json!({
                "lines": [
                    line_val(0, 0, 0, 10),
                    line_val(1, 0, 0, 10),
                    line_val(0, 5, 2, 10),
                    line_val(3, 1, 0, 10)
                ]
            }),
        ),
        _ => unreachable!("edge_case index exhausted"),
    }
}

fn generate(seed: u64, cases: usize, out: &Path) -> Result<()> {
    let mut rng = SplitMix64::new(seed);
    let total_weight: u32 = FAMILIES.iter().map(|f| f.2).sum();
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let file = std::fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
    let mut w = std::io::BufWriter::new(file);
    for i in 0..cases {
        let (code, op) = if i % 9 == 0 {
            ("edg".to_string(), edge_case(i / 9).0)
        } else {
            let mut pick = rng.below(total_weight as u64);
            let mut chosen = &FAMILIES[0];
            for fam in FAMILIES {
                if pick < fam.2 as u64 {
                    chosen = fam;
                    break;
                }
                pick -= fam.2 as u64;
            }
            let ops = chosen.1;
            (
                chosen.0.to_string(),
                ops[rng.below(ops.len() as u64) as usize],
            )
        };
        let args = if code == "edg" {
            edge_case(i / 9).1
        } else {
            build_args(&mut rng, op)
        };
        let case = json!({"id": format!("{code}-{i:06}"), "op": op, "args": args});
        serde_json::to_writer(&mut w, &case).context("serializing case")?;
        w.write_all(b"\n").context("writing case line")?;
    }
    w.flush().context("flushing case file")?;
    println!("wrote {cases} cases (seed {seed}) to {}", out.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Golden capture (Java oracle)
// ---------------------------------------------------------------------------

fn golden(cases: &Path, out: &Path) -> Result<()> {
    let repo_root = crate::oracle::find_repo_root()?;
    let cases = resolve_input(cases);
    let java = crate::oracle::resolve_java()?;
    let jar = crate::oracle::jar_path(&repo_root);
    anyhow::ensure!(
        jar.is_file(),
        "oracle jar missing at {} — build it once with `./gradlew executableJar`",
        jar.display()
    );
    let oracle_src = repo_root.join("rust/harness/oracle/GeometryCorpusOracle.java");
    anyhow::ensure!(
        oracle_src.is_file(),
        "oracle evaluator missing at {}",
        oracle_src.display()
    );
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut child = std::process::Command::new(&java)
        .arg("-cp")
        .arg(&jar)
        .arg(&oracle_src)
        .arg(&cases)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning {} with the oracle evaluator", java.display()))?;
    // The jar's logger writes warnings to stdout; only result lines start
    // with `{"id"` — everything else is JVM noise and is dropped here.
    let stdout = child
        .stdout
        .take()
        .context("evaluator stdout not captured")?;
    let out_file =
        std::fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
    let mut sink = std::io::BufWriter::new(out_file);
    let mut kept = 0usize;
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line.context("reading evaluator stdout")?;
        if line.starts_with("{\"id\"") {
            writeln!(sink, "{line}").context("writing golden line")?;
            kept += 1;
        }
    }
    // Draining stderr only after the stdout loop is safe: the evaluator
    // serializes per-case exceptions into the result JSON and never prints
    // them, so its stderr cannot fill up and deadlock a healthy run.
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stderr);
    }
    let status = child.wait().context("waiting for the oracle evaluator")?;
    if !status.success() {
        bail!(
            "oracle evaluator failed with {status}:\n{}",
            stderr.trim_end()
        );
    }
    println!(
        "wrote goldens for {kept} case(s) to {} (java: {})",
        out.display(),
        java.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// epic-geometry evaluation + comparison
// ---------------------------------------------------------------------------

fn args_pt(args: &Value, key: &str) -> (i32, i32) {
    let arr = args[key]
        .as_array()
        .unwrap_or_else(|| panic!("case arg {key} must be an int pair"));
    (
        i32::try_from(arr[0].as_i64().expect("case coord")).expect("coord fits i32"),
        i32::try_from(arr[1].as_i64().expect("case coord")).expect("coord fits i32"),
    )
}

fn args_int_point(args: &Value, key: &str) -> IntPoint {
    let (x, y) = args_pt(args, key);
    IntPoint::new(x, y)
}

fn args_point(args: &Value, key: &str) -> Point {
    let (x, y) = args_pt(args, key);
    // The oracle builds args with RAW `new IntPoint(x, y)` (trap T23):
    // Point::get_instance would silently box a RationalPoint above
    // +-CRIT_INT and diverge (corpus case ipt-000140).
    Point::Int(IntPoint::new(x, y))
}

fn args_vec(args: &Value, key: &str) -> Vector {
    let (x, y) = args_pt(args, key);
    Vector::get_instance(x, y)
}

fn args_i64(args: &Value, key: &str) -> i64 {
    args[key]
        .as_i64()
        .unwrap_or_else(|| panic!("case arg {key} must be an integer"))
}

fn args_f64_bits(args: &Value, key: &str) -> f64 {
    f64::from_bits(args_i64(args, key) as u64)
}

fn args_oct(args: &Value, key: &str, normalize: bool) -> IntOctagon {
    let arr = args[key]
        .as_array()
        .unwrap_or_else(|| panic!("case arg {key} must be an 8-int array"));
    let v: Vec<i32> = arr
        .iter()
        .map(|x| i32::try_from(x.as_i64().expect("oct coord")).expect("coord fits i32"))
        .collect();
    let oct = IntOctagon::new(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]);
    if normalize { oct.normalize() } else { oct }
}

fn args_box(args: &Value, key: &str) -> IntBox {
    let arr = args[key]
        .as_array()
        .unwrap_or_else(|| panic!("case arg {key} must be a 4-int array"));
    let c = |i: usize| i32::try_from(arr[i].as_i64().expect("box coord")).expect("coord fits i32");
    IntBox::new(IntPoint::new(c(0), c(1)), IntPoint::new(c(2), c(3)))
}

/// Line arg: `{"a": {"x", "y"}, "b": {"x", "y"}}` — an IntPoint line.
fn args_line(args: &Value, key: &str) -> Line {
    let obj = &args[key];
    let (ax, ay) = (
        obj["a"]["x"].as_i64().expect("line x"),
        obj["a"]["y"].as_i64().expect("line y"),
    );
    let (bx, by) = (
        obj["b"]["x"].as_i64().expect("line x"),
        obj["b"]["y"].as_i64().expect("line y"),
    );
    let ip = |x: i64, y: i64| {
        IntPoint::new(
            i32::try_from(x).expect("coord fits i32"),
            i32::try_from(y).expect("coord fits i32"),
        )
    };
    Line::new(Point::Int(ip(ax, ay)), Point::Int(ip(bx, by)))
}

fn args_lines(args: &Value, key: &str) -> Vec<Line> {
    args[key]
        .as_array()
        .unwrap_or_else(|| panic!("case arg {key} must be a line array"))
        .iter()
        .map(|l| args_line(&json!({"l": l}), "l"))
        .collect()
}

fn args_fpt(args: &Value, key: &str) -> FloatPoint {
    let arr = args[key].as_array().expect("float point");
    FloatPoint::new(
        f64::from_bits(arr[0].as_i64().expect("bits") as u64),
        f64::from_bits(arr[1].as_i64().expect("bits") as u64),
    )
}

fn args_fline(args: &Value, key: &str) -> FloatLine {
    let obj = &args[key];
    FloatLine::new(args_fpt(obj, "a"), args_fpt(obj, "b"))
}

fn pt_out(p: &Point) -> Value {
    match p {
        Point::Int(ip) => json!({"kind": "IntPoint", "v": [ip.x, ip.y]}),
        Point::Rational(rp) => {
            if rp.is_infinite() {
                json!({"kind": "Infinity"})
            } else {
                // to_float() is exact for this op table ONLY because every
                // emitted RationalPoint arises from 45-degree-direction cross
                // products whose denominators divide 2. If arbitrary-slope
                // lines ever feed these ops, switch to exact-rational
                // serialization (fields are package-private Java / pub(crate)
                // Rust today) — mirrored at ptOut in GeometryCorpusOracle.java.
                let f = rp.to_float();
                json!({"kind": "RationalPoint", "x_bits": f.x.to_bits() as i64, "y_bits": f.y.to_bits() as i64})
            }
        }
    }
}

fn fpt_out(p: FloatPoint) -> Value {
    json!({"kind": "FloatPoint", "x_bits": p.x.to_bits() as i64, "y_bits": p.y.to_bits() as i64})
}

/// Java emits Side ops as `strOut(sideOut(...))` — wrapped in `{"v": …}`.
fn side_value(s: Side) -> Value {
    match s {
        Side::Positive => json!({"v": "left"}),
        Side::Collinear => json!({"v": "collinear"}),
        Side::Negative => json!({"v": "right"}),
    }
}

/// Same wrapping for Signum ops (`{"v": signumOut(...)}`).
fn signum_value(s: Side) -> Value {
    match s {
        Side::Positive => json!({"v": "pos"}),
        Side::Collinear => json!({"v": "zero"}),
        Side::Negative => json!({"v": "neg"}),
    }
}

fn cmp_out(o: Ordering) -> Value {
    json!({"cmp": match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }})
}

/// Java `dblOut`: doubles always as `{"bits": i64}` (raw bit pattern).
fn bits_out(v: f64) -> Value {
    json!({"bits": v.to_bits() as i64})
}

fn v_out<T: serde::Serialize>(v: T) -> Value {
    json!({"v": v})
}

fn vec_out(v: &Vector) -> Value {
    match v {
        Vector::Int(iv) => json!({"kind": "IntVector", "v": [iv.x, iv.y]}),
        Vector::Rational(rv) => {
            json!({"kind": "RationalVector", "x": rv.x.to_string(), "y": rv.y.to_string(), "z": rv.z.to_string()})
        }
    }
}

fn dir_out(d: &Direction) -> Value {
    let v = d.get_vector();
    let v_field = match &v {
        Vector::Int(iv) => json!([iv.x, iv.y]),
        Vector::Rational(rv) => json!(format!("{}/{}/{}", rv.x, rv.y, rv.z)),
    };
    json!({"kind": "Direction", "v": v_field})
}

fn box_out(b: &IntBox) -> Value {
    // Emptiness is FIELD equality against the sentinel on BOTH sides — see
    // boxIsEmpty/octIsEmpty in GeometryCorpusOracle.java: a union of empties
    // returns a fresh sentinel-valued object, breaking Java reference
    // identity (the T2 port contract replaced identity with field equality).
    json!({
        "kind": "IntBox",
        "ll": [b.ll.x, b.ll.y],
        "ur": [b.ur.x, b.ur.y],
        "empty": *b == IntBox::EMPTY
    })
}

fn oct_out(o: &IntOctagon) -> Value {
    // Same field-equality emptiness contract as box_out above.
    json!({
        "kind": "IntOctagon",
        "v": [
            o.left_x, o.bottom_y, o.right_x, o.top_y,
            o.upper_left_diagonal_x, o.lower_right_diagonal_x,
            o.lower_left_diagonal_x, o.upper_right_diagonal_x
        ],
        "empty": *o == IntOctagon::EMPTY
    })
}

fn line_out(l: &Line) -> Value {
    json!({"kind": "Line", "a": pt_out(&l.a), "b": pt_out(&l.b)})
}

fn simplex_out(s: &Simplex) -> Value {
    let lines: Vec<Value> = (0..s.border_line_count())
        .map(|i| line_out(&s.border_line(i as i32)))
        .collect();
    json!({"kind": "Simplex", "lines": lines})
}

fn tile_out(t: &TileShape) -> Value {
    match t {
        TileShape::RegularTileShape(RegularTileShape::IntBox(b)) => box_out(b),
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(o)) => oct_out(o),
        TileShape::Simplex(s) => simplex_out(s),
    }
}

fn seg_out(s: &LineSegment) -> Value {
    json!({
        "kind": "LineSegment",
        "start": line_out(s.get_start_closing_line()),
        "middle": line_out(s.get_line()),
        "end": line_out(s.get_end_closing_line())
    })
}

fn poly_out(p: &Polyline) -> Value {
    let lines: Vec<Value> = p.lines.iter().map(line_out).collect();
    json!({"kind": "Polyline", "lines": lines})
}

fn null_out() -> Value {
    json!({"kind": "Null"})
}

fn tileshape_of_oct(o: IntOctagon) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntOctagon(o))
}

fn tileshape_of_box(b: IntBox) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(b))
}

/// Evaluates one corpus case through epic-geometry, producing the same JSON
/// tree the Java oracle emits for the same case. Panics on malformed cases
/// are caught by `compare` and reported as mismatches, never crash it.
pub fn eval_op(op: &str, args: &Value) -> Value {
    match op {
        "intpoint.determinant" => {
            v_out(args_int_point(args, "a").determinant(&args_int_point(args, "b")))
        }
        "intpoint.difference_by" => {
            vec_out(&args_point(args, "a").difference_by(&args_point(args, "b")))
        }
        "intpoint.fortyfive_degree_projection" => pt_out(&Point::Int(
            args_int_point(args, "a").fortyfive_degree_projection(&args_int_point(args, "b")),
        )),
        "intpoint.perpendicular_projection" => {
            pt_out(&args_line(args, "line").perpendicular_projection(&args_point(args, "p")))
        }
        "intpoint.surrounding_octagon" => oct_out(&args_int_point(args, "a").surrounding_octagon()),
        "vector.side_of" => side_value(args_vec(args, "a").side_of(&args_vec(args, "b"))),
        "vector.projection" => signum_value(args_vec(args, "a").projection(&args_vec(args, "b"))),
        "vector.turn_90_degree" => {
            vec_out(&args_vec(args, "a").turn_90_degree(args_i64(args, "factor") as i32))
        }
        "direction.get_instance" => dir_out(&Direction::get_instance(&args_vec(args, "v"))),
        "direction.turn_45_degree" => {
            let d = Direction::get_instance(&args_vec(args, "v"));
            dir_out(&d.turn_45_degree(args_i64(args, "factor") as i32))
        }
        "direction.compare_to" => cmp_out(
            Direction::get_instance(&args_vec(args, "a"))
                .compare_to(&Direction::get_instance(&args_vec(args, "b"))),
        ),
        "point.compare_xy" => cmp_out(
            args_point(args, "a")
                .compare_xy(&args_point(args, "b"))
                .cmp(&0),
        ),
        "point.translate_by" => pt_out(&args_point(args, "a").translate_by(&args_vec(args, "v"))),
        "intbox.intersection" => box_out(&args_box(args, "a").intersection(&args_box(args, "b"))),
        "intbox.union" => box_out(&args_box(args, "a").union(&args_box(args, "b"))),
        "intbox.offset_double" => {
            box_out(&args_box(args, "a").offset(args_f64_bits(args, "dist_bits")))
        }
        "intbox.area" => bits_out(args_box(args, "a").area()),
        "intbox.circumference" => bits_out(args_box(args, "a").circumference()),
        "intbox.intersects" => v_out(args_box(args, "a").intersects(&args_box(args, "b"))),
        "intbox.cutout" => {
            // Java `a.cutout((TileShape) b)` is `b.cutoutFrom(a)` — pieces of
            // a outside b. Rust `x.cutout_from(d)` is pieces of d outside x,
            // so the operands swap; IntBox.simplify() is identity on Java.
            let pieces: Vec<Value> = args_box(args, "b")
                .cutout_from(&args_box(args, "a"))
                .iter()
                .map(box_out)
                .collect();
            json!({"kind": "TileShapeList", "v": pieces})
        }
        "octagon.normalize" => oct_out(&args_oct(args, "a", false).normalize()),
        "octagon.intersection" => {
            oct_out(&args_oct(args, "a", true).intersection(&args_oct(args, "b", true)))
        }
        "octagon.union" => oct_out(&args_oct(args, "a", true).union(&args_oct(args, "b", true))),
        "octagon.offset" => {
            oct_out(&args_oct(args, "a", true).offset(args_f64_bits(args, "dist_bits")))
        }
        "octagon.intersects" => {
            v_out(args_oct(args, "a", true).intersects(&args_oct(args, "b", true)))
        }
        "octagon.overlaps" => v_out(args_oct(args, "a", true).overlaps(&args_oct(args, "b", true))),
        "octagon.contains_point" => {
            let p = args_point(args, "p");
            v_out(tileshape_of_oct(args_oct(args, "a", true)).contains_point(&p))
        }
        "octagon.contains_float_point" => {
            v_out(args_oct(args, "a", true).contains(&args_fpt(args, "p")))
        }
        "octagon.area" => bits_out(args_oct(args, "a", true).area()),
        "octagon.bounding_box" => {
            box_out(&tileshape_of_oct(args_oct(args, "a", true)).bounding_box())
        }
        "octagon.corner" => pt_out(&Point::Int(
            args_oct(args, "a", true).corner(args_i64(args, "no") as i32),
        )),
        "octagon.enlarge" => {
            oct_out(&args_oct(args, "a", true).enlarge(args_f64_bits(args, "offset_bits")))
        }
        "octagon.compare_edge" => side_value(
            args_oct(args, "a", true)
                .compare(&args_oct(args, "b", true), args_i64(args, "edge") as i32),
        ),
        "octagon.side_of_border_line" => side_value(args_oct(args, "a", true).side_of_border_line(
            args_i64(args, "x") as i32,
            args_i64(args, "y") as i32,
            args_i64(args, "no") as i32,
        )),
        "line.get_instance" => line_out(&Line::get_instance(
            args_point(args, "a"),
            Direction::get_instance(&args_vec(args, "dir")),
        )),
        "line.intersection" => match args_line(args, "a").intersection(&args_line(args, "b")) {
            Some(p) => pt_out(&p),
            None => json!({"kind": "Infinity"}),
        },
        "line.intersection_approx" => {
            fpt_out(args_line(args, "a").intersection_approx(&args_line(args, "b")))
        }
        "line.side_of_point" => side_value(args_line(args, "a").side_of(&args_point(args, "p"))),
        "line.side_of_intersection" => side_value(
            args_line(args, "a")
                .side_of_intersection(&args_line(args, "l1"), &args_line(args, "l2")),
        ),
        "line.perpendicular_projection" => {
            pt_out(&args_line(args, "a").perpendicular_projection(&args_point(args, "p")))
        }
        "line.fast_equals" => v_out(args_line(args, "a").fast_equals(&args_line(args, "b"))),
        "line.compare_to" => cmp_out(args_line(args, "a").compare_to(&args_line(args, "b"))),
        "line.direction" => dir_out(args_line(args, "a").direction()),
        // Java: `{"fbits": Float.floatToRawIntBits(length)}`.
        "line.length" => json!({"fbits": args_line(args, "a").length().to_bits() as i32}),
        "line_segment.intersection" => {
            let a = LineSegment::new(
                args_line(args, "s"),
                args_line(args, "m"),
                args_line(args, "e"),
            );
            let b = LineSegment::new(
                args_line(args, "s2"),
                args_line(args, "m2"),
                args_line(args, "e2"),
            );
            let cuts: Vec<Value> = a.intersection(&b).iter().map(line_out).collect();
            json!({"kind": "LineList", "v": cuts})
        }
        "line_segment.bounding_box" => box_out(
            &LineSegment::new(
                args_line(args, "s"),
                args_line(args, "m"),
                args_line(args, "e"),
            )
            .bounding_box(),
        ),
        "line_segment.sort_endpoints_in_xy" => seg_out(
            &LineSegment::new(
                args_line(args, "s"),
                args_line(args, "m"),
                args_line(args, "e"),
            )
            .sort_endpoints_in_xy(),
        ),
        "tileshape.from_8_ints" => {
            let v: Vec<i32> = args["v"]
                .as_array()
                .expect("8-int array")
                .iter()
                .map(|x| i32::try_from(x.as_i64().expect("int")).expect("fits i32"))
                .collect();
            oct_out(&TileShape::from_8_ints(
                v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7],
            ))
        }
        "tileshape.intersection_box_oct" => tile_out(
            &tileshape_of_box(args_box(args, "box"))
                .intersection(&tileshape_of_oct(args_oct(args, "oct", true))),
        ),
        "tileshape.from_points" => {
            let arr = args["points"].as_array().expect("points array");
            // Raw IntPoint args to mirror the oracle's `points()` helper
            // (trap T23 — NOT Point::get_instance).
            let pts: Vec<Point> = arr
                .iter()
                .map(|p| {
                    Point::Int(IntPoint::new(
                        i32::try_from(p[0].as_i64().expect("int")).expect("fits i32"),
                        i32::try_from(p[1].as_i64().expect("int")).expect("fits i32"),
                    ))
                })
                .collect();
            tile_out(&TileShape::from_points(&pts))
        }
        "simplex.get_instance" => simplex_out(&Simplex::get_instance(&args_lines(args, "lines"))),
        "simplex.intersection" => simplex_out(
            &Simplex::get_instance(&args_lines(args, "lines"))
                .intersection_simplex(&Simplex::get_instance(&args_lines(args, "lines2"))),
        ),
        "polyline.ctor" => poly_out(&Polyline::new(args_lines(args, "lines"))),
        "polyline.bounding_box" => {
            box_out(&Polyline::new(args_lines(args, "lines")).bounding_box_total())
        }
        "polyline.length_approx" => {
            bits_out(Polyline::new(args_lines(args, "lines")).length_approx_total())
        }
        "polyline.offset_shape" => {
            match Polyline::new(args_lines(args, "lines")).offset_shape(
                args_i64(args, "half_width") as i32,
                args_i64(args, "no") as i32,
            ) {
                Some(t) => tile_out(&t),
                None => null_out(),
            }
        }
        "polyline.is_multiple_of_45_degree" => {
            v_out(Polyline::new(args_lines(args, "lines")).is_multiple_of_45_degree())
        }
        "circle.intersects_octagon" => {
            let c = epic_geometry::circle::Circle::new(
                args_int_point(args, "center"),
                args_i64(args, "radius") as i32,
            );
            v_out(c.intersects_octagon(&args_oct(args, "oct", true)))
        }
        "circle.bounding_octagon" => {
            let c = epic_geometry::circle::Circle::new(
                args_int_point(args, "center"),
                args_i64(args, "radius") as i32,
            );
            oct_out(&c.bounding_octagon())
        }
        "floatpoint.round" => pt_out(&Point::Int(args_fpt(args, "p").round())),
        "floatpoint.round_to_grid" => pt_out(&Point::Int(
            args_fpt(args, "p")
                .round_to_grid(args_i64(args, "h") as i32, args_i64(args, "v") as i32),
        )),
        "floatpoint.distance_square" => {
            bits_out(args_fpt(args, "a").distance_square(&args_fpt(args, "b")))
        }
        "floatpoint.inside_circle" => v_out(args_fpt(args, "p").inside_circle(
            &args_fpt(args, "p1"),
            &args_fpt(args, "p2"),
            &args_fpt(args, "p3"),
        )),
        "floatline.intersection" => {
            match args_fline(args, "a").intersection(&args_fline(args, "b")) {
                Some(p) => fpt_out(p),
                None => null_out(),
            }
        }
        "floatline.projection" => {
            fpt_out(args_fline(args, "a").perpendicular_projection(&args_fpt(args, "p")))
        }
        _ => panic!("evaluator op table drift: {op}"),
    }
}

// ---------------------------------------------------------------------------
// Compare
// ---------------------------------------------------------------------------

fn read_jsonl(path: &Path) -> Result<Vec<Value>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut out = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line.with_context(|| format!("reading {}", path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(trimmed)
                .with_context(|| format!("parsing {} line: {trimmed}", path.display()))?,
        );
    }
    Ok(out)
}

/// Evaluates all cases with epic-geometry and diffs against the Java goldens.
/// Prints the first 20 mismatches; exits nonzero on any mismatch or count
/// divergence. Per-case Rust panics are reported as mismatches, never crash.
pub fn compare(cases_path: &Path, golden_path: &Path) -> Result<()> {
    let cases = read_jsonl(&resolve_input(cases_path))?;
    let golden_path = resolve_input(golden_path);
    let golden_lines = read_jsonl(&golden_path)?;
    let mut golden: HashMap<String, Value> = HashMap::with_capacity(golden_lines.len());
    for line in &golden_lines {
        let id = line["id"]
            .as_str()
            .context("golden line without id")?
            .to_string();
        let out = line
            .get("out")
            .cloned()
            .context(format!("golden line {id} without out"))?;
        golden.insert(id, out);
    }

    // Quality-review T17b M-Q2: the shared lock + RAII silence. This
    // is the SECOND hook participant in the epic-harness test binary
    // (run_detail_pass's watchdog pins being the first): without the
    // lock, a corpus silence-up window racing a watchdog swap can
    // permanently leak a silenced hook — the sentinel then fails with
    // its assertion message swallowed. In the BIN process the lock is
    // uncontended (this is the only participant).
    let _hook_lock = crate::panic_hook::lock();
    let _silence = crate::panic_hook::Silenced::new();
    let mut mismatches: Vec<String> = Vec::new();
    let mut total_mismatches = 0usize;
    for case in &cases {
        let id = case["id"].as_str().context("case without id")?;
        let op = case["op"].as_str().context("case without op")?;
        let args = case.get("args").cloned().unwrap_or(Value::Null);
        let actual = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| eval_op(op, &args)))
            .unwrap_or_else(|_| json!({"kind": "RustPanic"}));
        let failure = match golden.get(id) {
            None => Some(format!(
                "FAIL {op} {id}\n  golden: <missing from golden file>\n  rust:   {actual}"
            )),
            Some(expected) if expected != &actual => Some(format!(
                "FAIL {op} {id}\n  golden: {expected}\n  rust:   {actual}"
            )),
            _ => None,
        };
        if let Some(failure) = failure {
            total_mismatches += 1;
            if mismatches.len() < 20 {
                mismatches.push(failure);
            }
        }
    }
    // The RAII guard restores the hook on every exit path from here on.

    if golden.len() != cases.len() {
        bail!(
            "corpus compare: count mismatch — {} case(s) vs {} golden line(s) in {}",
            cases.len(),
            golden.len(),
            golden_path.display()
        );
    }
    if total_mismatches == 0 {
        println!("PASS {}/{}", cases.len(), cases.len());
        Ok(())
    } else {
        println!(
            "FAIL {}/{} matched",
            cases.len() - total_mismatches,
            cases.len()
        );
        bail!(
            "corpus compare: {total_mismatches} mismatch(es):\n{}{}",
            mismatches.join("\n"),
            if mismatches.len() == 20 {
                "\n… (further mismatches suppressed)"
            } else {
                ""
            }
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("epic-corpus-{name}-{}.jsonl", std::process::id()))
    }

    /// The generator must be a pure function of the seed.
    #[test]
    fn generate_is_deterministic_per_seed() {
        let a = temp_path("det-a");
        let b = temp_path("det-b");
        generate(7, 60, &a).expect("generate a");
        generate(7, 60, &b).expect("generate b");
        let da = std::fs::read(&a).expect("read a");
        let db = std::fs::read(&b).expect("read b");
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
        assert_eq!(da, db, "same seed must produce byte-identical corpora");
    }

    /// The committed corpus artifact is pinned: any change to the corpus
    /// definition (family weights, edge-case stride, RNG) must regenerate
    /// `corpus/geometry-cases.jsonl` in the same commit. Mutation-proven
    /// necessity: `i % 9 == 0` -> `i % 8 == 0` rewrote the corpus and passed
    /// the old self-consistency-only suite. Task 10 completed the pair with
    /// the matching compare-vs-committed-golden pin
    /// ([`Self::compare_matches_committed_golden`]).
    #[test]
    fn generate_matches_committed_corpus() {
        let committed =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus/geometry-cases.jsonl");
        let expected =
            std::fs::read(&committed).expect("committed corpus/geometry-cases.jsonl must exist");
        let tmp = temp_path("committed-pin");
        generate(20260912, 5000, &tmp).expect("generate the committed corpus");
        let actual = std::fs::read(&tmp).expect("read generated corpus");
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(
            actual, expected,
            "generate(seed 20260912, 5000 cases) diverged from the committed corpus — \
             regenerate the artifact in the same commit as the generator change"
        );
    }

    /// Task 9 review follow-up (landed Task 10): the committed GOLDEN is
    /// pinned by running the pure-Rust evaluator against it — the corpus
    /// gate needs no JDK. Regenerating either artifact is only sanctioned
    /// through the generate -> golden -> compare flow, in one commit.
    #[test]
    fn compare_matches_committed_golden() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus");
        compare(
            &dir.join("geometry-cases.jsonl"),
            &dir.join("geometry-golden.jsonl"),
        )
        .expect("committed corpus must compare green against the committed golden");
    }

    /// Rust-side pins captured live from the Java oracle smoke (GeometryCorpus
    /// Oracle.java over the 7 hand-written cases); the Rust evaluator must
    /// produce the very same JSON trees.
    #[test]
    fn eval_op_matches_java_captured_goldens() {
        let crit = CRIT_INT;
        let cases: Vec<(&str, Value, Value)> = vec![
            (
                "line.intersection",
                json!({"a": {"a": {"x": 0, "y": 0}, "b": {"x": 10, "y": 0}}, "b": {"a": {"x": 0, "y": 5}, "b": {"x": 10, "y": 5}}}),
                json!({"kind": "Infinity"}),
            ),
            (
                "line.intersection",
                json!({"a": {"a": {"x": 0, "y": 0}, "b": {"x": 10, "y": 10}}, "b": {"a": {"x": 0, "y": 10}, "b": {"x": 10, "y": 0}}}),
                json!({"kind": "IntPoint", "v": [5, 5]}),
            ),
            (
                "intbox.intersection",
                json!({"a": [0, 0, 10, 10], "b": [20, 20, 30, 30]}),
                json!({"kind": "IntBox", "ll": [crit, crit], "ur": [-crit, -crit], "empty": true}),
            ),
            (
                "floatpoint.round",
                json!({"p": [4612811918334230528i64, 0]}),
                json!({"kind": "IntPoint", "v": [3, 0]}),
            ),
            (
                "intpoint.determinant",
                json!({"a": [46341, 0], "b": [0, 46341]}),
                json!({"v": 2147488281i64}),
            ),
            (
                "floatline.intersection",
                json!({
                    "a": {"a": [0, 0], "b": [4607182418800017408i64, 0]},
                    "b": {"a": [0, 4613937818241073152i64], "b": [4613937818241073152i64, 4613937818241073152i64]}
                }),
                json!({"kind": "Null"}),
            ),
            (
                "octagon.intersection",
                json!({"a": [0, 1000, 0, 700, -500, 1500, -500, 1500], "b": [500, 2000, 300, 900, -500, 2500, -200, 2000]}),
                json!({
                    "kind": "IntOctagon",
                    "v": [crit, crit, -crit, -crit, crit, -crit, crit, -crit],
                    "empty": true
                }),
            ),
        ];
        for (op, args, expected) in cases {
            let actual = eval_op(op, &args);
            assert_eq!(
                actual, expected,
                "eval_op({op}) diverged from the Java-captured golden"
            );
        }
    }

    /// A hand-broken golden line must be reported as exactly that one case,
    /// and the intact pair must pass — compare is the smoke gate.
    #[test]
    fn compare_reports_exactly_the_broken_case() {
        let cases_path = temp_path("cmp-cases");
        let golden_path = temp_path("cmp-golden");
        let cases = format!(
            "{}\n{}\n",
            json!({"id": "lin-000001", "op": "line.intersection", "args": {"a": {"a": {"x": 0, "y": 0}, "b": {"x": 10, "y": 10}}, "b": {"a": {"x": 0, "y": 10}, "b": {"x": 10, "y": 0}}}}),
            json!({"id": "lin-000002", "op": "line.intersection", "args": {"a": {"a": {"x": 0, "y": 0}, "b": {"x": 10, "y": 0}}, "b": {"a": {"x": 0, "y": 5}, "b": {"x": 10, "y": 5}}}}),
        );
        std::fs::write(&cases_path, cases).expect("write cases");
        let good = format!(
            "{}\n{}\n",
            json!({"id": "lin-000001", "out": {"kind": "IntPoint", "v": [5, 5]}}),
            json!({"id": "lin-000002", "out": {"kind": "Infinity"}}),
        );
        std::fs::write(&golden_path, &good).expect("write goldens");
        assert!(
            compare(&cases_path, &golden_path).is_ok(),
            "intact pair must pass"
        );

        let broken = format!(
            "{}\n{}\n",
            json!({"id": "lin-000001", "out": {"kind": "IntPoint", "v": [6, 5]}}),
            json!({"id": "lin-000002", "out": {"kind": "Infinity"}}),
        );
        assert_ne!(broken, good, "the broken golden must actually differ");
        std::fs::write(&golden_path, broken).expect("write broken goldens");
        let err = compare(&cases_path, &golden_path).expect_err("broken golden must fail");
        let _ = std::fs::remove_file(&cases_path);
        let _ = std::fs::remove_file(&golden_path);
        let msg = format!("{err:#}");
        assert!(
            msg.contains("lin-000001"),
            "failure must name the broken case: {msg}"
        );
        assert!(
            !msg.contains("lin-000002"),
            "the intact case must not be reported: {msg}"
        );
    }
}
