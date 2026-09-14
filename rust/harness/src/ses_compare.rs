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
