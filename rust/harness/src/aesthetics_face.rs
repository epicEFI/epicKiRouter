//! The M8-T1 reference door (java-free): parse a routed/reference DSN
//! through the drc_corpus parse path (`epic_dsn::read_board` +
//! `Board::from_ses_board` — the reinsert normalization is NOT applied
//! here: the measurer is a pure `&Board` read surface and never
//! queries the searchtree, so the tree fill would be dead state on
//! this path), run the measurer, and print the metrics JSON to
//! stdout — the EXACT bytes a golden carries. Never a gate; the
//! fixture-evidence instrument for the aesthetics population and the
//! golden producer.
//!
//! Invocation: `epic-harness aesthetics --dsn <path>`; exits nonzero
//! on a parse failure. stdout carries ONLY the JSON (the evidence
//! convention wraps the command, not the artifact).

use anyhow::{Result, bail};
use epic_dsn::reader::{DsnReadResult, read_board};
use std::path::Path;

/// The golden directory (repo-relative) — committed NEW artifacts,
/// justified in the commit message.
#[cfg(test)]
pub const GOLDEN_DIR: &str = "rust/harness/fixtures/aesthetics/golden";

/// Parses the DSN and prints the metrics JSON (the golden bytes).
///
/// # Errors
///
/// A missing file or a parse failure is a hard error (exit 1).
pub fn run(dsn: &Path) -> Result<String> {
    let bytes = std::fs::read(dsn)
        .map_err(|error| anyhow::anyhow!("cannot read design {}: {error}", dsn.display()))?;
    let mut ses = epic_dsn::ses_board::SesBoard::new();
    match read_board(&bytes, &mut ses) {
        DsnReadResult::Success { warnings } | DsnReadResult::OutlineMissing { warnings } => {
            for warning in warnings {
                eprintln!("Warning: {warning}");
            }
        }
        DsnReadResult::ParseError { location, detail } => {
            bail!("parse error at {location}: {detail}");
        }
        DsnReadResult::IoError => {
            bail!("I/O error reading {}", dsn.display());
        }
    }
    let board = epic_board::board::Board::from_ses_board(&ses);
    let metrics = epic_board::aesthetics::aesthetics_metrics(&board, board.rules());
    let json = epic_board::aesthetics::render_json(&metrics);
    println!("{json}");
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The golden-verify pin: every committed golden re-derives
    /// byte-identically from the corresponding reference board,
    /// IN-PROCESS (no spawned bin — the stale-bin false-green family,
    /// cerebrum 17, never applies). Skips loudly empty when the
    /// golden dir is absent so a bare `cargo test` on a fresh clone
    /// before the fixture commit is a clean pass.
    #[test]
    fn t1_goldens_rederive_byte_identically() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let golden_dir = repo_root.join(GOLDEN_DIR);
        if !golden_dir.is_dir() {
            panic!(
                "golden dir missing: {} — the committed sample must exist",
                golden_dir.display()
            );
        }
        let mut checked = 0usize;
        let mut names: Vec<String> = std::fs::read_dir(&golden_dir)
            .expect("golden dir readable")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert!(!names.is_empty(), "no goldens committed");
        for name in &names {
            let board_name = name.strip_suffix(".json").expect("json golden name");
            let dsn = repo_root
                .join("scripts/benchmark/fixtures/PCBench")
                .join(board_name)
                .join("reference-routed.dsn");
            let json = run(&dsn).expect("reference board parses");
            let golden = std::fs::read_to_string(golden_dir.join(name)).expect("golden readable");
            assert_eq!(
                json + "\n",
                golden,
                "golden {name} does not re-derive byte-identically"
            );
            checked += 1;
        }
        // The committed sample is 21 + 3 tier-context boards — a
        // silently truncated golden dir must not verify as complete.
        assert_eq!(
            checked, 24,
            "the committed goldens: 21 sample + 3 tier context"
        );
    }
}
