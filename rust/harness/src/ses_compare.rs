//! SES emission-parity corpus (M1b Task 13): `dsn ses-golden` captures the
//! Java oracle's parse→emit session bytes per tier A+B fixture (this file,
//! Part B3); Part B4 adds the sexpr canonical compare (`dsn ses-compare`)
//! that gates the Rust port's [`epic_dsn::ses::writer`] against them.
//!
//! ## Golden naming (plan `:286`, interpreted)
//!
//! The plan words the artifact as `rust/harness/corpus/ses/<stem>.ses.golden`
//! — but tier B contains TWO fixtures with the stem `unrouted.dsn`
//! (`PCBench/1-Wire-Wing-pcb_1-Wire_Wing/unrouted.dsn` and
//! `PCBench/1Bitsy_1bitsy/unrouted.dsn`; digest ids dsn-0019/dsn-0020), so
//! a bare-stem scheme would silently overwrite one golden with the other
//! during capture. [`golden_name`] therefore flattens the
//! fixtures_root-relative path with `__`:
//! `DAC2020_boards/DAC2020_bm01.dsn` → `DAC2020_boards__DAC2020_bm01.ses.golden`.
//! The naming is pinned (two `unrouted` fixtures → two distinct names) in
//! [`pins`].
//!
//! ## designName semantics
//!
//! `SesWriter.write(board, out, designName)` threads the design FILE NAME
//! (not the full `-de` path, not the board's `(pcb ...)` name) — proven by
//! the real session `fixtures/Issue313-FastTest.ses`, which opens with
//! `(session Issue313-FastTest.ses` / `(base_design Issue313-FastTest.dsn`)
//! and by the `saveAsSpecctraSessionSes` call sites. The capture passes
//! each fixture's file name on BOTH sides (the oracle manifest carries it
//! explicitly; the B4 compare derives it the same way here).
//!
//! ## Capture discipline
//!
//! ONE JVM per run (D14): `rust/harness/oracle/SesEmitOracle.java` parses
//! each fixture via `DsnReader.readBoard` (the same null-observer smoke
//! path as `DsnParseOracle`) and emits the parse-time board through the
//! jar's `SesWriter` into `<flattened>.ses.golden`, one flushed result
//! line per case. The Rust side rewrites the whole corpus directory each
//! capture (all 20 tier A+B fixtures, one JVM, seconds) and removes stale
//! `*.ses.golden` files not in the expected set — the committed directory
//! is always exactly the expected inventory.

use std::collections::HashSet;
use std::io::BufRead;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::tiers::TierFile;

/// The tiers the SES goldens cover (plan: "Tier A+B canonical compare").
pub const SES_TIERS: [&str; 2] = ["A", "B"];

/// 11 tier-A + 9 tier-B fixtures (tiers.yaml; pinned so a tiers.yaml edit
/// that silently drops/adds fixtures fails the inventory pin loudly).
pub const PINNED_TIER_AB_COUNT: usize = 20;

/// One capture case — also the exact JVM-manifest line shape
/// (`SesEmitOracle` reads id/path/design/golden as strings).
#[derive(Debug, Clone, Serialize)]
struct SesCase {
    id: String,
    /// Repo-relative fixture path (resolved against the oracle's
    /// repo-root working directory).
    path: String,
    /// The design FILE NAME — the designName both writers receive.
    design: String,
    /// The golden file name inside the capture/corpus directory.
    golden: String,
}

/// The golden artifact name: the fixtures_root-relative path, `/` → `__`,
/// `.dsn` stripped, `.ses.golden` appended (module docs: the `unrouted.dsn`
/// collision that rules out bare stems).
fn golden_name(fixtures_root_rel: &str) -> String {
    let stem = fixtures_root_rel
        .strip_suffix(".dsn")
        .unwrap_or(fixtures_root_rel);
    format!("{}.ses.golden", stem.replace('/', "__"))
}

/// Builds the tier A+B case list in tiers.yaml order (`ses-NNNN` ids) and
/// enforces golden-name uniqueness — with `__` flattening a duplicate can
/// only come from the same path listed twice, which would double-capture
/// one fixture and strand the other's golden.
fn build_cases(tier_file: &TierFile) -> Result<Vec<SesCase>> {
    let mut cases = Vec::new();
    for tier in &tier_file.tiers {
        if !SES_TIERS.contains(&tier.name.as_str()) {
            continue;
        }
        for fixture in &tier.fixtures {
            let path = format!("{}/{}", tier_file.fixtures_root.display(), fixture.path);
            let design = Path::new(&fixture.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| fixture.path.clone());
            cases.push(SesCase {
                id: format!("ses-{:04}", cases.len() + 1),
                path,
                design,
                golden: golden_name(&fixture.path),
            });
        }
    }
    anyhow::ensure!(
        !cases.is_empty(),
        "tiers {:?} select no fixtures — tiers.yaml drifted?",
        SES_TIERS
    );
    let mut seen = HashSet::new();
    for case in &cases {
        anyhow::ensure!(
            seen.insert(case.golden.as_str()),
            "golden name {} collides (fixture {} listed twice?)",
            case.golden,
            case.path
        );
    }
    Ok(cases)
}

/// `dsn ses-golden`: run `SesEmitOracle` (one JVM) over the tier A+B
/// fixtures and rewrite `rust/harness/corpus/ses/` to exactly the captured
/// goldens (stale `*.ses.golden` files outside the expected inventory are
/// removed and reported).
pub fn golden(repo_root: &Path, tiers: &Path, out: &Path, jvm_xmx: &str) -> Result<()> {
    let started = Instant::now();
    let tiers_path = crate::dsn_corpus::resolve_input(repo_root, tiers);
    let tier_file = TierFile::load(&tiers_path)?;
    let cases = build_cases(&tier_file)?;
    // Runtime mirror of the inventory pin: a tiers.yaml edit that silently
    // drops or adds tier A+B fixtures must fail the capture loudly, not
    // commit a different-sized corpus than the pinned one.
    anyhow::ensure!(
        cases.len() == PINNED_TIER_AB_COUNT,
        "tier A+B selected {} fixture(s) — the pinned corpus size is {} (tiers.yaml drifted?)",
        cases.len(),
        PINNED_TIER_AB_COUNT
    );

    let java = crate::oracle::resolve_java()?;
    let jar = crate::oracle::jar_path(repo_root);
    anyhow::ensure!(
        jar.is_file(),
        "oracle jar missing at {} — build it once with `./gradlew executableJar`",
        jar.display()
    );
    let oracle_src = repo_root.join("rust/harness/oracle/SesEmitOracle.java");
    anyhow::ensure!(
        oracle_src.is_file(),
        "oracle evaluator missing at {}",
        oracle_src.display()
    );

    // Fresh capture dir: this process's own id namespaces it; a leftover
    // from a crashed earlier run must never leak files into this capture.
    let capture_dir = std::env::temp_dir().join(format!("epic-ses-capture-{}", std::process::id()));
    if capture_dir.exists() {
        std::fs::remove_dir_all(&capture_dir)
            .with_context(|| format!("cleaning {}", capture_dir.display()))?;
    }
    std::fs::create_dir_all(&capture_dir)
        .with_context(|| format!("creating {}", capture_dir.display()))?;
    let jvm_manifest =
        std::env::temp_dir().join(format!("epic-ses-manifest-{}.jsonl", std::process::id()));
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&jvm_manifest)
            .with_context(|| format!("creating {}", jvm_manifest.display()))?;
        for case in &cases {
            writeln!(
                file,
                "{}",
                serde_json::to_string(case).expect("ses case serializes")
            )
            .with_context(|| format!("writing {}", jvm_manifest.display()))?;
        }
    }
    let stderr_path =
        std::env::temp_dir().join(format!("epic-ses-oracle-stderr-{}.log", std::process::id()));

    let mut child = std::process::Command::new(&java)
        .arg(format!("-Xmx{jvm_xmx}"))
        // Locale-pinned like `dsn golden`: locale-sensitive identifier
        // handling must not capture different goldens on a non-en host.
        .arg("-Duser.language=en")
        .arg("-Duser.country=US")
        .arg("-cp")
        .arg(&jar)
        .arg(&oracle_src)
        .arg(&jvm_manifest)
        .arg(&capture_dir)
        .current_dir(repo_root)
        .stdout(std::process::Stdio::piped())
        .stderr(
            std::fs::File::create(&stderr_path)
                .with_context(|| format!("creating {}", stderr_path.display()))?,
        )
        .spawn()
        .with_context(|| format!("spawning {} with the SES emit oracle", java.display()))?;

    // Only result lines start with `{"id"` — FRLogger warnings write to
    // stdout in between and are dropped here. BYTE-wise read: a fixture
    // that makes FRLogger echo control bytes would abort UTF-8 `lines()`
    // mid-run (the dsn-golden lesson).
    let stdout = child.stdout.take().context("oracle stdout not captured")?;
    let mut fresh_lines = Vec::new();
    {
        let mut reader = std::io::BufReader::new(stdout);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            let read = reader
                .read_until(b'\n', &mut raw)
                .context("reading oracle stdout")?;
            if read == 0 {
                break;
            }
            if raw.starts_with(b"{\"id\"") {
                let line = String::from_utf8_lossy(&raw);
                fresh_lines.push(line.trim_end_matches(['\n', '\r']).to_string());
            }
        }
    }
    drop(child.stderr.take());
    let status = child.wait().context("waiting for the oracle")?;
    let _ = std::fs::remove_file(&jvm_manifest);
    if !status.success() {
        let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
        let _ = std::fs::remove_file(&stderr_path);
        bail!(
            "oracle failed with {status} (captured {}/{} result line(s) before failure):\n{}",
            fresh_lines.len(),
            cases.len(),
            stderr.trim_end()
        );
    }
    let stderr_bytes = std::fs::metadata(&stderr_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if stderr_bytes > 0 {
        println!(
            "note: oracle stderr held {stderr_bytes} byte(s) of FRLogger noise on a successful capture (discarded)"
        );
    }
    let _ = std::fs::remove_file(&stderr_path);

    anyhow::ensure!(
        fresh_lines.len() == cases.len(),
        "oracle produced {} result line(s) for {} case(s) — truncated capture must not be committed",
        fresh_lines.len(),
        cases.len()
    );
    #[derive(serde::Deserialize)]
    struct OracleResult {
        id: String,
        #[serde(default)]
        result: String,
        #[serde(default)]
        sha256: Option<String>,
    }
    let mut results = Vec::with_capacity(cases.len());
    for (index, line) in fresh_lines.iter().enumerate() {
        let record: OracleResult = serde_json::from_str(line)
            .with_context(|| format!("parsing oracle result line {line}"))?;
        anyhow::ensure!(
            record.id == cases[index].id,
            "oracle returned id {} for case {} — machinery bug",
            record.id,
            cases[index].id
        );
        results.push(record);
    }
    // Every tier A+B fixture is a known parse Success (digest goldens
    // dsn-0001..0020): any non-Success now is an oracle/jar regression,
    // not a corpus condition to carry — bail loudly instead of committing
    // a hole. EmitError likewise (an emission bug must never enter the
    // corpus as anything other than a hard failure).
    for (case, record) in cases.iter().zip(&results) {
        anyhow::ensure!(
            record.result == "Success",
            "{} ({}) produced result {} — expected Success for every tier A+B fixture",
            case.id,
            case.path,
            record.result
        );
    }

    // Verify each captured golden against the oracle's own sha256 before
    // it enters the corpus (catches a truncated/partial file write), then
    // rewrite the corpus directory to exactly the expected inventory.
    let out_dir = crate::dsn_corpus::resolve_output(repo_root, out);
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let expected: HashSet<&str> = cases.iter().map(|case| case.golden.as_str()).collect();
    let mut removed_stale = Vec::new();
    for entry in
        std::fs::read_dir(&out_dir).with_context(|| format!("reading {}", out_dir.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", out_dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".ses.golden") && !expected.contains(name.as_str()) {
            std::fs::remove_file(entry.path())
                .with_context(|| format!("removing stale {}", entry.path().display()))?;
            removed_stale.push(name);
        }
    }
    let mut total_bytes = 0u64;
    for (case, record) in cases.iter().zip(&results) {
        let captured = capture_dir.join(&case.golden);
        anyhow::ensure!(
            captured.is_file(),
            "oracle reported Success but wrote no golden {} for {}",
            case.golden,
            case.path
        );
        let bytes = std::fs::read(&captured)
            .with_context(|| format!("reading captured {}", captured.display()))?;
        let sha = sha256_hex(&bytes);
        anyhow::ensure!(
            Some(sha.as_str()) == record.sha256.as_deref(),
            "captured golden {} does not match the oracle's sha256 — partial write?",
            case.golden
        );
        std::fs::write(out_dir.join(&case.golden), &bytes)
            .with_context(|| format!("writing {}", out_dir.join(&case.golden).display()))?;
        total_bytes += bytes.len() as u64;
    }
    let _ = std::fs::remove_dir_all(&capture_dir);
    if !removed_stale.is_empty() {
        println!(
            "removed {} stale golden(s): {}",
            removed_stale.len(),
            removed_stale.join(", ")
        );
    }
    println!(
        "captured {} SES golden(s) ({} byte(s)) into {} in {:.1}s (java: {})",
        cases.len(),
        total_bytes,
        out_dir.display(),
        started.elapsed().as_secs_f64(),
        java.display()
    );
    Ok(())
}

/// SHA-256 as lowercase hex (cross-check against the oracle's digest) —
/// the same provider the parse digest uses.
fn sha256_hex(bytes: &[u8]) -> String {
    crate::dsn_digest::sha256_hex(bytes)
}

// ---------------------------------------------------------------------------
// Canonical compare (Task 13 B4): sexpr token trees + byte-diff
// classification, java-free (CI-able).
// ---------------------------------------------------------------------------

/// One parsed token of session text: an atom (identifier, number, or a
/// `"…"`-delimited string KEPT VERBATIM including its quotes) or a
/// parenthesized list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sexpr {
    Atom(String),
    List(Vec<Sexpr>),
}

/// Nesting cap: a pathological document must fail as a clean anyhow
/// error, not abort the process on stack overflow (committed goldens sit
/// at depth ~8; the port's own emission is similarly shallow).
const MAX_SEXPR_DEPTH: usize = 512;

/// Parses ONE root sexpr (a session document). Whitespace only separates
/// tokens; atoms compare textually, so `1250` and `1250.0` are DIFFERENT
/// atoms — the "numbers exact" rule. Only `"` gets string lexing: the
/// session's reduced `(parser` scope never re-declares `string_quote`,
/// so every corpus quote is the default `"`; a non-`"` quote char lexes
/// as a plain atom character on BOTH sides, which still compares
/// textually equal.
fn parse_sexpr(text: &str) -> Result<Sexpr> {
    let bytes = text.as_bytes();
    let mut pos = 0usize;
    let root = parse_node(text, bytes, &mut pos, 0)?;
    skip_ws(bytes, &mut pos);
    anyhow::ensure!(
        pos == bytes.len(),
        "trailing text after the root sexpr at byte {pos}"
    );
    Ok(root)
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && bytes[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}

fn parse_node(text: &str, bytes: &[u8], pos: &mut usize, depth: usize) -> Result<Sexpr> {
    anyhow::ensure!(
        depth <= MAX_SEXPR_DEPTH,
        "sexpr nesting exceeds {MAX_SEXPR_DEPTH} levels — not a session document"
    );
    skip_ws(bytes, pos);
    anyhow::ensure!(*pos < bytes.len(), "unexpected end of input");
    if bytes[*pos] == b'(' {
        *pos += 1;
        let mut items = Vec::new();
        loop {
            skip_ws(bytes, pos);
            anyhow::ensure!(*pos < bytes.len(), "unterminated list");
            if bytes[*pos] == b')' {
                *pos += 1;
                return Ok(Sexpr::List(items));
            }
            items.push(parse_node(text, bytes, pos, depth + 1)?);
        }
    }
    if bytes[*pos] == b')' {
        bail!("unbalanced ')' at byte {pos}");
    }
    if bytes[*pos] == b'"' {
        // Quoted atom, kept verbatim with its quotes.
        let start = *pos;
        *pos += 1;
        while *pos < bytes.len() && bytes[*pos] != b'"' {
            *pos += 1;
        }
        anyhow::ensure!(
            *pos < bytes.len(),
            "unterminated quoted atom starting at byte {start}"
        );
        *pos += 1;
        return Ok(Sexpr::Atom(text[start..*pos].to_string()));
    }
    let start = *pos;
    while *pos < bytes.len()
        && !bytes[*pos].is_ascii_whitespace()
        && !matches!(bytes[*pos], b'(' | b')' | b'"')
    {
        *pos += 1;
    }
    anyhow::ensure!(*pos > start, "empty atom at byte {start}");
    Ok(Sexpr::Atom(text[start..*pos].to_string()))
}

/// True for number-shaped atoms: optional sign, digits, `.`, `e`/`E`.
/// Quoted atoms are never numeric (numbers are never quoted). Identifiers
/// like `1X08` or `F.Cu` fail the scan (X/F/C/u) — only true numerics
/// pass, which is what the T40 coordinate classifier needs.
fn is_number_atom(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('"')
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
}

/// One canonical divergence between two session trees.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Divergence {
    /// Path to the diverging position, e.g.
    /// `session/routes[2]/wire[1]/path[0]/atom[4]`.
    path: String,
    /// Compact renderings of the two nodes at that position.
    golden: String,
    rust: String,
    /// Both sides are numeric atoms (the T40-snappability precondition).
    numeric: bool,
    /// The position sits inside a `(path …`/`(polyline_path …` scope —
    /// the WIRE-COORDINATE locus (T40 snaps wire endpoints only; a via
    /// or place coordinate diff is genuine).
    in_path_scope: bool,
}

/// A one-line node rendering: atoms verbatim, lists as
/// `<list <head> ×n>`.
fn describe(node: &Sexpr) -> String {
    match node {
        Sexpr::Atom(atom) => atom.clone(),
        Sexpr::List(items) => {
            let head = match items.first() {
                Some(Sexpr::Atom(name)) => name.clone(),
                _ => "?".to_string(),
            };
            format!("<list {head} ×{}>", items.len())
        }
    }
}

fn list_head(items: &[Sexpr]) -> &str {
    match items.first() {
        Some(Sexpr::Atom(name)) => name.as_str(),
        _ => "list",
    }
}

/// Walks both trees in parallel, pushing one [`Divergence`] per differing
/// position (atoms, list-length mismatches, structure swaps), at most
/// `limit` of them. `in_path_scope` turns true while descending into
/// `(path`/`(polyline_path` scopes — their direct children are the wire
/// coordinates (width first, then corner pairs).
fn collect_divergences(
    golden: &Sexpr,
    rust: &Sexpr,
    path: &str,
    in_path_scope: bool,
    out: &mut Vec<Divergence>,
    limit: usize,
) {
    if out.len() >= limit {
        return;
    }
    match (golden, rust) {
        (Sexpr::Atom(g), Sexpr::Atom(r)) => {
            if g != r {
                out.push(Divergence {
                    path: path.to_string(),
                    golden: g.clone(),
                    rust: r.clone(),
                    numeric: is_number_atom(g) && is_number_atom(r),
                    in_path_scope,
                });
            }
        }
        (Sexpr::List(g), Sexpr::List(r)) => {
            if g.len() != r.len() {
                out.push(Divergence {
                    path: path.to_string(),
                    golden: describe(golden),
                    rust: describe(rust),
                    numeric: false,
                    in_path_scope,
                });
            }
            let common = g.len().min(r.len());
            for index in 0..common {
                if out.len() >= limit {
                    return;
                }
                let child_path = format!("{path}/{}[{index}]", list_head(g));
                // Entering a path scope marks every child below it as a
                // wire-coordinate position.
                let child_scope = in_path_scope || matches!(list_head(g), "path" | "polyline_path");
                collect_divergences(&g[index], &r[index], &child_path, child_scope, out, limit);
            }
        }
        (golden, rust) => {
            out.push(Divergence {
                path: path.to_string(),
                golden: describe(golden),
                rust: describe(rust),
                numeric: false,
                in_path_scope,
            });
        }
    }
}

/// The plan's byte-diff classes: `normalization` = bytes differ but the
/// trees are equal (whitespace only); `T40-snap` = every divergence is a
/// numeric atom inside a wire path scope (the documented endpoint-snap
/// omission — expected ZERO on parse-time boards, which have no contacts);
/// `genuine` = anything else (a port bug).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteDiffClass {
    Normalization,
    T40Snap,
    Genuine,
}

fn classify(divergences: &[Divergence]) -> ByteDiffClass {
    if divergences.is_empty() {
        ByteDiffClass::Normalization
    } else if divergences
        .iter()
        .all(|divergence| divergence.numeric && divergence.in_path_scope)
    {
        ByteDiffClass::T40Snap
    } else {
        ByteDiffClass::Genuine
    }
}

/// (differing byte positions, first differing offset) over two byte
/// strings — the normalization report line.
fn byte_diff_stats(golden: &[u8], rust: &[u8]) -> (usize, Option<usize>) {
    let mut count = golden.len().abs_diff(rust.len());
    let mut first = None;
    for (offset, (g, r)) in golden.iter().zip(rust.iter()).enumerate() {
        if g != r {
            count += 1;
            if first.is_none() {
                first = Some(offset);
            }
        }
    }
    if first.is_none() && count > 0 {
        // One is a strict prefix of the other: the first extra byte.
        first = Some(golden.len().min(rust.len()));
    }
    (count, first)
}

/// The max divergences reported per fixture (report clarity, not a
/// comparator limit — classification always sees EVERY divergence via a
/// full-depth pass with the report pass's limit disabled).
const REPORT_DIVERGENCE_LIMIT: usize = 8;

/// `dsn ses-compare`: parse every tier A+B fixture with epic-dsn, emit
/// the session through the port, and compare against the committed
/// goldens — canonical (whitespace-insensitive, numbers exact) plus the
/// byte-diff classification. Java-free.
pub fn compare(repo_root: &Path, tiers: &Path, golden: &Path) -> Result<()> {
    let started = Instant::now();
    let tiers_path = crate::dsn_corpus::resolve_input(repo_root, tiers);
    let tier_file = TierFile::load(&tiers_path)?;
    let cases = build_cases(&tier_file)?;
    anyhow::ensure!(
        cases.len() == PINNED_TIER_AB_COUNT,
        "tier A+B selected {} fixture(s) — the pinned corpus size is {} (tiers.yaml drifted?)",
        cases.len(),
        PINNED_TIER_AB_COUNT
    );
    let golden_dir = crate::dsn_corpus::resolve_input(repo_root, golden);
    anyhow::ensure!(
        golden_dir.is_dir(),
        "SES golden directory missing at {} — run `dsn ses-golden` first",
        golden_dir.display()
    );

    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    let mut byte_equal = 0usize;
    let mut normalization = 0usize;
    let mut t40_snap: Vec<&str> = Vec::new();
    let mut genuine: Vec<&str> = Vec::new();
    for case in &cases {
        let bytes = std::fs::read(repo_root.join(&case.path))
            .with_context(|| format!("reading fixture {} ({})", case.path, case.id))?;
        let mut board = SesBoard::new();
        let rust_text = match read_board(&bytes, &mut board) {
            DsnReadResult::Success { .. } => {
                epic_dsn::ses::writer::write_session(&board, &case.design)
            }
            other => bail!(
                "{} ({}) parsed as {other:?} — every tier A+B fixture is a known Success",
                case.id,
                case.path
            ),
        };
        let golden_bytes = std::fs::read(golden_dir.join(&case.golden))
            .with_context(|| format!("reading golden {} ({})", case.golden, case.id))?;
        if rust_text.as_bytes() == golden_bytes.as_slice() {
            byte_equal += 1;
            println!("{} {}: byte-equal", case.id, case.golden);
            continue;
        }
        let golden_tree = parse_sexpr(&String::from_utf8_lossy(&golden_bytes))
            .with_context(|| format!("parsing golden {}", case.golden))?;
        let rust_tree = parse_sexpr(&rust_text)
            .with_context(|| format!("parsing the Rust emission for {}", case.path))?;
        // Full-depth pass for classification (no limit), limited pass for
        // the printed report.
        let mut all = Vec::new();
        collect_divergences(
            &golden_tree,
            &rust_tree,
            "session",
            false,
            &mut all,
            usize::MAX,
        );
        let mut reported = Vec::new();
        collect_divergences(
            &golden_tree,
            &rust_tree,
            "session",
            false,
            &mut reported,
            REPORT_DIVERGENCE_LIMIT,
        );
        match classify(&all) {
            ByteDiffClass::Normalization => {
                normalization += 1;
                let (count, first) = byte_diff_stats(&golden_bytes, rust_text.as_bytes());
                println!(
                    "{} {}: canonical-equal (normalization; {count} differing byte(s), first at offset {first:?})",
                    case.id, case.golden
                );
            }
            ByteDiffClass::T40Snap => {
                t40_snap.push(case.id.as_str());
                println!(
                    "{} {}: T40-SNAP ({} divergence(s), wire-coordinate numerics only):",
                    case.id,
                    case.golden,
                    all.len()
                );
                for divergence in &reported {
                    println!(
                        "  {} golden={} rust={}",
                        divergence.path, divergence.golden, divergence.rust
                    );
                }
            }
            ByteDiffClass::Genuine => {
                genuine.push(case.id.as_str());
                println!(
                    "{} {}: GENUINE ({} divergence(s)):",
                    case.id,
                    case.golden,
                    all.len()
                );
                for divergence in &reported {
                    println!(
                        "  {} golden={} rust={}",
                        divergence.path, divergence.golden, divergence.rust
                    );
                }
            }
        }
    }

    println!(
        "ses-compare: {} fixture(s) — {byte_equal} byte-equal, {normalization} normalization, \
         {} T40-snap, {} genuine — in {:.1}s (java-free)",
        cases.len(),
        t40_snap.len(),
        genuine.len(),
        started.elapsed().as_secs_f64()
    );
    // Both canonical-diff classes fail: genuine is a port bug, and T40-snap
    // is expected ZERO on parse-time boards (no contacts exist pre-routing,
    // so the Java oracle's snappedEndpoint also always returns null there —
    // module docs of the writer). A T40 firing means that analysis broke.
    if !t40_snap.is_empty() {
        bail!(
            "T40-snap diffs on {t40_snap:?}: parse-time boards have no contacts, so the documented \
             endpoint-snap omission cannot bite here — investigate before exiting M1b"
        );
    }
    if !genuine.is_empty() {
        bail!(
            "ses-compare: {} genuine divergence(s) on {genuine:?} — port bugs, fix before exit",
            genuine.len()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pins (java-free: tiers.yaml + the committed corpus directory only). The
// harness is a binary crate, so integration tests cannot import its code —
// the M1b convention is in-module `pins`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        // CARGO_MANIFEST_DIR = <repo>/rust/harness.
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .expect("repo root is two levels above the harness crate")
    }

    fn committed_tier_file() -> TierFile {
        TierFile::load(&repo_root().join("rust/harness/config/tiers.yaml"))
            .expect("committed tiers.yaml loads")
    }

    fn corpus_dir() -> PathBuf {
        repo_root().join("rust/harness/corpus/ses")
    }

    fn committed_goldens() -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(corpus_dir())
            .expect("committed ses corpus dir exists")
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".ses.golden"))
            .collect();
        names.sort();
        names
    }

    /// The committed corpus directory is EXACTLY the tier A+B expected
    /// inventory (20 goldens): a missing capture, a renamed fixture
    /// without recapture, or a stale leftover golden all fail here with
    /// the offending names.
    #[test]
    fn golden_inventory_matches_tiers_ab() {
        let cases = build_cases(&committed_tier_file()).expect("tier A+B cases");
        assert_eq!(cases.len(), PINNED_TIER_AB_COUNT, "tier A+B fixture count");
        let mut expected: Vec<&str> = cases.iter().map(|case| case.golden.as_str()).collect();
        expected.sort_unstable();
        let committed = committed_goldens();
        let expected: Vec<String> = expected.into_iter().map(String::from).collect();
        assert_eq!(
            committed, expected,
            "committed SES goldens must be exactly the tier A+B inventory"
        );
    }

    /// The naming collision that forced `__` flattening: the two tier-B
    /// `unrouted.dsn` fixtures (distinct directories, identical stem) map
    /// to DISTINCT golden names — a regression to bare-stem naming fails
    /// here even before the inventory pin's set mismatch.
    #[test]
    fn golden_names_disambiguate_the_two_unrouted_fixtures() {
        let cases = build_cases(&committed_tier_file()).expect("tier A+B cases");
        let unrouted: Vec<&SesCase> = cases
            .iter()
            .filter(|case| case.path.ends_with("/unrouted.dsn"))
            .collect();
        assert_eq!(
            unrouted.len(),
            2,
            "tier B is expected to carry exactly two unrouted.dsn fixtures"
        );
        assert_ne!(unrouted[0].golden, unrouted[1].golden);
        // Both carry the SAME design name (the file name) — the collision
        // is in the artifact name only, never in the emitted header.
        assert_eq!(unrouted[0].design, "unrouted.dsn");
        assert_eq!(unrouted[1].design, "unrouted.dsn");
        for case in unrouted {
            assert!(
                case.golden.starts_with("PCBench__"),
                "flattened name keeps the parent directory: {}",
                case.golden
            );
        }
    }

    /// Each committed golden is byte-shaped like a session: opens with
    /// `(session `, closes with `)` with NO trailing newline, and carries
    /// the fixture's design file name in `(base_design `. These are
    /// content pins on the committed artifacts (java-free); the B4
    /// canonical compare is the full gate.
    #[test]
    fn committed_goldens_are_session_shaped() {
        let cases = build_cases(&committed_tier_file()).expect("tier A+B cases");
        assert_eq!(cases.len(), committed_goldens().len());
        for case in &cases {
            let bytes = std::fs::read(corpus_dir().join(&case.golden))
                .unwrap_or_else(|e| panic!("reading {}: {e}", case.golden));
            assert!(
                bytes.starts_with(b"(session "),
                "{} must open with '(session ' (quoted or bare name follows)",
                case.golden
            );
            assert_eq!(
                bytes.last(),
                Some(&b')'),
                "{} must end with ')' — no trailing newline (jar-verified layout)",
                case.golden
            );
            assert!(
                !bytes.ends_with(b"\n"),
                "{} must not end with a newline",
                case.golden
            );
            let text = String::from_utf8_lossy(&bytes);
            // The design name is quoted iff the T38 identifier rule says
            // so (`_` is reserved: "DAC2020_bm01.dsn" is written quoted) —
            // accept both spellings, anchored at the scope keyword.
            let bare = format!("(base_design {}", case.design);
            let quoted = format!("(base_design \"{}\"", case.design);
            assert!(
                text.contains(&bare) || text.contains(&quoted),
                "{} must carry the design name {:?} in (base_design …",
                case.golden,
                case.design
            );
        }
    }

    /// [`golden_name`] pure pins: directory flattening, the `.dsn` strip,
    /// and the no-suffix passthrough (a path without `.dsn` keeps its
    /// name; only the suffix rule differs from a bare-stem scheme).
    #[test]
    fn golden_name_flattens_directories() {
        assert_eq!(
            golden_name("DAC2020_boards/DAC2020_bm01.dsn"),
            "DAC2020_boards__DAC2020_bm01.ses.golden"
        );
        assert_eq!(
            golden_name("pic_programmer.dsn"),
            "pic_programmer.ses.golden"
        );
        assert_eq!(
            golden_name("KiCad_10_demos/sonde xilinx.dsn"),
            "KiCad_10_demos__sonde xilinx.ses.golden"
        );
        // A deep path keeps every segment — nothing is truncated away.
        assert_eq!(
            golden_name("PCBench/1-Wire-Wing-pcb_1-Wire_Wing/unrouted.dsn"),
            "PCBench__1-Wire-Wing-pcb_1-Wire_Wing__unrouted.ses.golden"
        );
        // No .dsn suffix: the name passes through untouched before the
        // .ses.golden append (tiers.yaml only lists .dsn paths today).
        assert_eq!(golden_name("board.txt"), "board.txt.ses.golden");
    }

    /// The design name is the FILE NAME, never the directory path — the
    /// `(base_design …)` header pin (Issue313 fixture evidence).
    #[test]
    fn design_name_is_the_file_name() {
        let cases = build_cases(&committed_tier_file()).expect("tier A+B cases");
        let sonde = cases
            .iter()
            .find(|case| case.path.contains("sonde"))
            .expect("tier A includes the sonde xilinx fixture");
        assert_eq!(sonde.design, "sonde xilinx.dsn");
        assert!(!sonde.design.contains('/'));
    }
}

/// B4 pins: the sexpr parser, the divergence collector, and EVERY
/// classifier branch — synthetic trees, no fixtures, no JVM (the
/// cerebrum pin rules: branch-executing, inputs where wrong and right
/// differ).
#[cfg(test)]
mod compare_pins {
    use super::*;

    fn atom(text: &str) -> Sexpr {
        Sexpr::Atom(text.to_string())
    }

    fn list(items: Vec<Sexpr>) -> Sexpr {
        Sexpr::List(items)
    }

    /// Nested lists, quoted atoms kept verbatim (quotes in the atom
    /// text), and numbers as plain atoms — the parser's structural
    /// contract in one tree.
    #[test]
    fn sexpr_parses_nested_lists_and_quoted_atoms() {
        let tree = parse_sexpr("(session \"a b\"\n  (net GND (wire (path F.Cu 1250\n 10 20))))")
            .expect("well-formed sexpr parses");
        assert_eq!(
            tree,
            list(vec![
                atom("session"),
                atom("\"a b\""),
                list(vec![
                    atom("net"),
                    atom("GND"),
                    list(vec![
                        atom("wire"),
                        list(vec![
                            atom("path"),
                            atom("F.Cu"),
                            atom("1250"),
                            atom("10"),
                            atom("20")
                        ]),
                    ]),
                ]),
            ])
        );
    }

    /// Whitespace-insensitivity (the normalization class's precondition):
    /// same tree from different spacing/newlines.
    #[test]
    fn sexpr_is_whitespace_insensitive() {
        let a = parse_sexpr("(a (b 1 2) c)").expect("parses");
        let b = parse_sexpr("  ( a\n\t( b   1\n2 )\nc )  ").expect("parses");
        assert_eq!(a, b);
    }

    /// Malformed input fails loudly: unterminated list, unbalanced `)`,
    /// trailing text after the root, empty input, unterminated quote.
    #[test]
    fn sexpr_rejects_malformed_input() {
        assert!(parse_sexpr("(a").is_err(), "unterminated list");
        assert!(parse_sexpr("a)").is_err(), "bare atom is trailing text");
        assert!(
            parse_sexpr("(a) (b)").is_err(),
            "two roots is trailing text"
        );
        assert!(parse_sexpr("(a) x").is_err(), "trailing atom");
        assert!(parse_sexpr("").is_err(), "empty input");
        assert!(parse_sexpr("\"abc").is_err(), "unterminated quote");
    }

    /// Numbers are EXACT atoms (the plan's "numbers exact"): `1250` and
    /// `1250.0` parse to different trees — a comparator that parsed and
    /// compared as floats would call them equal.
    #[test]
    fn numbers_are_exact_atoms() {
        let a = parse_sexpr("(path 1250 10)").expect("parses");
        let b = parse_sexpr("(path 1250.0 10)").expect("parses");
        assert_ne!(a, b);
        assert_eq!(a, list(vec![atom("path"), atom("1250"), atom("10")]));
    }

    /// The numeric scan: true numerics (signs, exponents, fractions)
    /// pass; identifiers containing digits (`1X08`), layer names
    /// (`F.Cu`), and quoted atoms fail.
    #[test]
    fn is_number_atom_scans_strictly() {
        assert!(is_number_atom("1250"));
        assert!(is_number_atom("-1037336"));
        assert!(is_number_atom("1250.0"));
        assert!(is_number_atom("1.25E-3"));
        assert!(!is_number_atom("1X08"), "digit-leading identifier");
        assert!(!is_number_atom("F.Cu"), "layer name");
        assert!(
            !is_number_atom("\"1250\""),
            "quoted atoms are never numeric"
        );
        assert!(!is_number_atom(""));
    }

    /// A coordinate change inside a `(path` scope: ONE divergence,
    /// numeric, path-scoped → T40Snap (the documented endpoint-snap
    /// class).
    #[test]
    fn wire_coordinate_diff_classifies_t40_snap() {
        let golden = parse_sexpr("(wire (path F.Cu 1250 10 20 30 40))").expect("parses");
        let rust = parse_sexpr("(wire (path F.Cu 1250 11 20 30 40))").expect("parses");
        let mut divergences = Vec::new();
        collect_divergences(&golden, &rust, "session", false, &mut divergences, 8);
        assert_eq!(divergences.len(), 1);
        let divergence = &divergences[0];
        assert_eq!(divergence.golden, "10");
        assert_eq!(divergence.rust, "11");
        assert!(divergence.numeric);
        assert!(divergence.in_path_scope);
        assert!(
            divergence.path.contains("path["),
            "the path context names the scope: {}",
            divergence.path
        );
        assert_eq!(classify(&divergences), ByteDiffClass::T40Snap);
    }

    /// `polyline_path` scopes are wire-coordinate scopes too.
    #[test]
    fn polyline_path_scope_is_t40_eligible() {
        let golden = parse_sexpr("(polyline_path F.Cu 1250 10 20)").expect("parses");
        let rust = parse_sexpr("(polyline_path F.Cu 1250 10 21)").expect("parses");
        let mut divergences = Vec::new();
        collect_divergences(&golden, &rust, "session", false, &mut divergences, 8);
        assert_eq!(divergences.len(), 1);
        assert!(divergences[0].numeric);
        assert!(divergences[0].in_path_scope);
        assert_eq!(classify(&divergences), ByteDiffClass::T40Snap);
    }

    /// A numeric diff OUTSIDE any path scope is GENUINE: T40 snaps wire
    /// endpoints only, so a `(place` or `(via` coordinate diff is a port
    /// bug.
    #[test]
    fn numeric_diff_outside_path_scope_is_genuine() {
        let golden = parse_sexpr("(place U7 100 200 front 0)").expect("parses");
        let rust = parse_sexpr("(place U7 101 200 front 0)").expect("parses");
        let mut divergences = Vec::new();
        collect_divergences(&golden, &rust, "session", false, &mut divergences, 8);
        assert_eq!(divergences.len(), 1);
        assert!(divergences[0].numeric);
        assert!(!divergences[0].in_path_scope);
        assert_eq!(classify(&divergences), ByteDiffClass::Genuine);
    }

    /// An identifier diff inside a path scope (layer name) is GENUINE —
    /// the class requires EVERY divergence to be numeric.
    #[test]
    fn identifier_diff_in_path_scope_is_genuine() {
        let golden = parse_sexpr("(wire (path F.Cu 1250 10 20))").expect("parses");
        let rust = parse_sexpr("(wire (path B.Cu 1250 10 20))").expect("parses");
        let mut divergences = Vec::new();
        collect_divergences(&golden, &rust, "session", false, &mut divergences, 8);
        assert_eq!(divergences.len(), 1);
        assert!(!divergences[0].numeric);
        assert_eq!(classify(&divergences), ByteDiffClass::Genuine);
    }

    /// Structural mismatches: an atom where a list belongs, and a
    /// list-length difference (an extra corner) — both GENUINE, with
    /// list summaries in the divergence report.
    #[test]
    fn structural_mismatches_are_genuine_with_summaries() {
        let golden = parse_sexpr("(wire (path F.Cu 1250 10 20 30 40))").expect("parses");
        // An atom swapped for the corner list.
        let swapped = parse_sexpr("(wire 1250)").expect("parses");
        let mut divergences = Vec::new();
        collect_divergences(&golden, &swapped, "session", false, &mut divergences, 8);
        assert_eq!(divergences.len(), 1);
        assert!(divergences[0].golden.contains("<list path"));
        assert_eq!(divergences[0].rust, "1250");
        assert_eq!(classify(&divergences), ByteDiffClass::Genuine);

        // An extra corner in the golden: length mismatch, reported once
        // with the list sizes in ATOMS (path, F.Cu, width, then 4 vs 8
        // coordinate atoms → ×7 vs ×9).
        let extra = parse_sexpr("(wire (path F.Cu 1250 10 20 30 40 50 60))").expect("parses");
        let mut divergences = Vec::new();
        collect_divergences(&golden, &extra, "session", false, &mut divergences, 8);
        // The length mismatch divergence (numeric=false ⇒ Genuine even
        // though every atom-level diff sits in the path scope).
        assert!(
            divergences
                .iter()
                .any(|d| d.golden.contains("×7") && d.rust.contains("×9")),
            "length mismatch reported with sizes: {divergences:?}"
        );
        assert_eq!(classify(&divergences), ByteDiffClass::Genuine);
    }

    /// Equal trees classify as Normalization regardless of the byte
    /// difference that got us here (the whitespace-only class).
    #[test]
    fn equal_trees_classify_normalization() {
        let golden = parse_sexpr("(session x (a 1))").expect("parses");
        let rust = parse_sexpr("(session\n  x\n  (a 1)\n)").expect("parses");
        let mut divergences = Vec::new();
        collect_divergences(&golden, &rust, "session", false, &mut divergences, 8);
        assert!(divergences.is_empty());
        assert_eq!(classify(&divergences), ByteDiffClass::Normalization);
    }

    /// The depth cap: pathological nesting fails as a clean anyhow error
    /// (not a stack-overflow abort), while realistic depth still parses.
    #[test]
    fn sexpr_depth_cap_fails_cleanly() {
        let deep_ok = format!("{}a{}", "(".repeat(400), ")".repeat(400));
        assert!(parse_sexpr(&deep_ok).is_ok(), "400 levels must parse");
        let too_deep = format!("{}a{}", "(".repeat(600), ")".repeat(600));
        let err = parse_sexpr(&too_deep).expect_err("600 levels must fail");
        assert!(
            err.to_string().contains("nesting"),
            "unexpected error: {err:#}"
        );
    }

    /// The report limit truncates without changing classification
    /// inputs: a limited pass stops at `limit`, a full pass sees all.
    /// Both x and y coordinates differ, so 20 corners = 40 divergences.
    #[test]
    fn report_limit_truncates_but_classify_sees_all() {
        let golden_text = format!(
            "(wire (path F.Cu 1250 {}))",
            (0..20)
                .map(|n| format!("{n} {n}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let rust_text = format!(
            "(wire (path F.Cu 1250 {}))",
            (0..20)
                .map(|n| format!("{} {}", n + 1, n + 2))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let golden = parse_sexpr(&golden_text).expect("parses");
        let rust = parse_sexpr(&rust_text).expect("parses");
        let mut limited = Vec::new();
        collect_divergences(&golden, &rust, "session", false, &mut limited, 5);
        assert_eq!(limited.len(), 5, "the report pass stops at the limit");
        let mut all = Vec::new();
        collect_divergences(&golden, &rust, "session", false, &mut all, usize::MAX);
        assert_eq!(all.len(), 40, "the full pass sees every divergence");
        assert!(all.iter().all(|d| d.numeric && d.in_path_scope));
        assert_eq!(classify(&all), ByteDiffClass::T40Snap);
    }

    /// Byte-diff stats: differing bytes, prefix extension (first extra
    /// byte at the shared length), and identical bytes.
    #[test]
    fn byte_diff_stats_first_offset_and_count() {
        // Plain mismatch.
        assert_eq!(byte_diff_stats(b"abc", b"abd"), (1, Some(2)));
        // Prefix: golden is a strict prefix of rust.
        assert_eq!(byte_diff_stats(b"ab", b"abXY"), (2, Some(2)));
        // Identical.
        assert_eq!(byte_diff_stats(b"same", b"same"), (0, None));
    }
}
