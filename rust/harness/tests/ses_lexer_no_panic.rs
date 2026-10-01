//! M1b Task 12 invariant: the epic-dsn reader never PANICS on session
//! files. Every `*.ses` under the repo-root `fixtures/` tree (39 at Task 12
//! time: 13 at the root plus 26 nested under issue directories) is fed to
//! [`read_board`]; the invariant is that the call RETURNS a
//! [`DsnReadResult`] variant — Success, OutlineMissing, ParseError, or
//! IoError — never unwinds. An `.ses` file is not a DSN, so ParseError is
//! the expected outcome for most of them; what must never happen is a
//! panic (index-out-of-bounds, integer overflow, bad UTF-8 slicing) leaking
//! out of the lexer/parser, because `read_board` is the safety boundary the
//! GUI and API feed arbitrary user files through.
//!
//! `catch_unwind` wraps each parse so the failing FILE is named in the
//! panic message instead of the test dying on the first offender with no
//! suspect identified.

use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;

/// `rust/harness` -> `rust` -> repo root (compile-time stable).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("harness dir has a parent")
        .parent()
        .expect("rust dir has a parent")
        .to_path_buf()
}

/// Deterministic recursive walk collecting every `*.ses` under `dir`,
/// sorted so the iteration order never depends on the OS directory order.
fn collect_ses_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|error| panic!("reading {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.expect("directory entry readable");
        let path = entry.path();
        if path.is_dir() {
            collect_ses_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("ses") {
            out.push(path);
        }
    }
}

#[test]
fn read_board_returns_a_variant_never_panics_on_ses_fixtures() {
    let mut ses_files = Vec::new();
    collect_ses_files(&repo_root().join("fixtures"), &mut ses_files);
    ses_files.sort();
    assert_eq!(
        ses_files.len(),
        39,
        "pinned .ses fixture count — update deliberately when fixtures change"
    );
    for path in &ses_files {
        let bytes =
            fs::read(path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
        let file = path.display().to_string();
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut board = SesBoard::new();
            read_board(&bytes, &mut board)
        }));
        let result = outcome.unwrap_or_else(|payload| {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<opaque panic payload>".to_string());
            panic!("read_board PANICKED on {file}: {detail}");
        });
        // The plan's Task 12 invariant is TWO-fold: no panic AND a clean
        // ParseError (an `.ses` is not a DSN — the pcb header dispatch must
        // reject it gracefully, exactly like Java's reader throws/returns
        // its error result). Exhaustive match so a future variant must be
        // consciously classified here.
        match result {
            DsnReadResult::ParseError { .. } => {}
            DsnReadResult::Success { .. }
            | DsnReadResult::OutlineMissing { .. }
            | DsnReadResult::IoError => {
                panic!("{file}: expected ParseError for a session file, got {result:?}")
            }
        }
    }
}
