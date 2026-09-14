//! M1b Task 11 regeneration pins over the COMMITTED corpus artifacts.
//! Deliberately fast: no fixture parsing, no JVM. The manifest
//! byte-identity pin (which needs the harness generator) lives in
//! `src/dsn_corpus.rs::pins` because the harness is a binary crate whose
//! code integration tests cannot import.

use std::path::{Path, PathBuf};

fn corpus_path(name: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/rust/harness.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root: &Path = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("repo root is two levels above the harness crate");
    root.join("rust/harness/corpus").join(name)
}

fn jsonl_lines(name: &str) -> Vec<serde_json::Value> {
    let raw = std::fs::read_to_string(corpus_path(name)).expect("committed corpus file");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("strict JSONL"))
        .collect()
}

/// (b) The committed golden has exactly 1,332 lines == the manifest's.
#[test]
fn golden_line_count_is_pinned_1332_and_matches_manifest() {
    let manifest = jsonl_lines("dsn-manifest.jsonl");
    let golden = jsonl_lines("dsn-golden.jsonl");
    assert_eq!(golden.len(), 1332, "pinned corpus size");
    assert_eq!(manifest.len(), 1332, "manifest corpus size");
}

/// (c) The golden's id sequence exactly equals the manifest's (no
/// missing, extra, or reordered entries).
#[test]
fn golden_ids_exactly_equal_manifest_ids() {
    let manifest = jsonl_lines("dsn-manifest.jsonl");
    let golden = jsonl_lines("dsn-golden.jsonl");
    let manifest_ids: Vec<&str> = manifest
        .iter()
        .map(|entry| entry["id"].as_str().expect("manifest id"))
        .collect();
    let golden_ids: Vec<&str> = golden
        .iter()
        .map(|record| record["id"].as_str().expect("golden id"))
        .collect();
    assert_eq!(manifest_ids, golden_ids);
}
