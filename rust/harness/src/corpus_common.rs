//! The shared corpus shell (M3 Task 1 — the M2 review carry-forward):
//! the JSONL mechanics around the per-corpus row types, unified out of
//! the triplicated dsn/index/undo corpus modules BEFORE M3's routing
//! corpus (T15/T16) becomes a fourth copy.
//!
//! What lives here: the manifest ENTRY row type, the byte format the
//! committed manifests share (one compact JSON object per line,
//! `\n`-terminated including the last), strict JSONL loading, the
//! positional manifest/corpus alignment guard, the diff-rendering
//! helpers (`json_string`, `truncate`), the one `sha256_hex` spelling
//! plus its non-hashing `hex` sibling, and the tier-then-stressor
//! fixture walk the index and undo manifests share.
//!
//! What deliberately does NOT: the golden RECORD types (dsn
//! `GoldenRecord`, index `GoldenRecord`, undo `UndoRecord` have
//! genuinely different fields — they stay with their corpora and wire
//! in via [`HasId`]), the per-corpus manifest SET policy (`build_manifest`
//! — dsn's two-set digest/soak policy is its own), and the
//! `diff_records` field walks. This is also NOT the M0 baseline
//! manifest schema — `manifest.rs` is a different concern (tier
//! fixture/sha baselines) that happens to share the name.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::tiers::TierFile;

// Lowercase SHA-256 hex (HASH the bytes, then hex): the canonical home
// stays `dsn_digest` (the digest module owns hashing;
// `ses_compare::sha256_hex` already delegated there — that is the
// precedent). This re-export is the one spelling the corpus modules
// use, so the undo `hex_sha256` copy died with the extraction.
pub use crate::dsn_digest::sha256_hex;

/// Lowercase hex of ALREADY-HASHED bytes — byte→hex ONLY, no hashing.
/// The distinct sibling of [`sha256_hex`]: the index corpus's tree
/// dumps hash incrementally (`Sha256::new` + `update` per line) and
/// then hex `finalize()`'s 32-byte output — feeding that into
/// `sha256_hex` would hash TWICE. (This is the former
/// `index_corpus::hex`; the extraction initially merged the two and
/// the 33/33 `index compare` gate caught it as a full-corpus
/// trees[*].sha256 divergence — the trap the M3 brief flagged.)
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// ---------------------------------------------------------------------------
// The manifest row
// ---------------------------------------------------------------------------

/// One manifest line:
/// `{"id":"<scheme>-NNNN","path":"<repo-relative posix>"}` — compact
/// JSON, `deny_unknown_fields`, field order id-then-path (the committed
/// manifest bytes depend on both). Shared by the dsn (`dsn-`/`soak-`),
/// index (`idx-`) and undo (`und-`) corpora; the id SCHEME and the
/// fixture-set policy are each corpus's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub id: String,
    pub path: String,
}

/// The positional id of one alignment-checked line: [`ManifestEntry`]
/// and every corpus's golden record carry one.
pub trait HasId {
    fn id(&self) -> &str;
}

impl HasId for ManifestEntry {
    fn id(&self) -> &str {
        &self.id
    }
}

// ---------------------------------------------------------------------------
// Manifest building (the shared walk + the committed byte format)
// ---------------------------------------------------------------------------

/// The fixture-set walk shared by the index and undo manifests (the
/// undo corpus deliberately routes the SAME fixture set — the same
/// boards exercise both surfaces): the tier fixtures in tiers.yaml
/// order, then every `rust/harness/fixtures/index-stress/*.dsn`
/// lexicographic, dedup by path. A pure function of the repository
/// tree; callers assign their own id scheme over the returned order.
pub fn tier_then_stressor_paths(repo_root: &Path) -> Result<Vec<String>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut rel = Vec::new();

    let tiers_path = repo_root.join("rust/harness/config/tiers.yaml");
    let tier_file = TierFile::load(&tiers_path)?;
    for tier in &tier_file.tiers {
        for fixture in &tier.fixtures {
            let path = format!("{}/{}", tier_file.fixtures_root.display(), fixture.path);
            if seen.insert(path.clone()) {
                rel.push(path);
            }
        }
    }

    let stress_dir = repo_root.join("rust/harness/fixtures/index-stress");
    let mut stress = Vec::new();
    for entry in std::fs::read_dir(&stress_dir)
        .with_context(|| format!("reading directory {}", stress_dir.display()))?
    {
        let entry = entry.with_context(|| format!("reading directory {}", stress_dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".dsn") {
            stress.push(format!("rust/harness/fixtures/index-stress/{name}"));
        }
    }
    stress.sort();
    for path in stress {
        if seen.insert(path.clone()) {
            rel.push(path);
        }
    }
    Ok(rel)
}

/// Serializes manifest entries exactly as committed: one compact JSON
/// object per line, `\n`-terminated (including the last).
pub fn manifest_bytes<E: Serialize>(entries: &[E]) -> Vec<u8> {
    let mut buf = Vec::new();
    for entry in entries {
        buf.extend_from_slice(
            serde_json::to_string(entry)
                .expect("manifest entry serialization cannot fail")
                .as_bytes(),
        );
        buf.push(b'\n');
    }
    buf
}

// ---------------------------------------------------------------------------
// Strict JSONL loading
// ---------------------------------------------------------------------------

/// Parses JSONL text strictly per line (blank lines skipped; a
/// malformed line fails naming `{source} line {n}`) — the in-memory
/// half of [`load_jsonl`], split so comparator pins can exercise
/// truncated/reordered corpora without touching the filesystem.
pub fn parse_jsonl<T: serde::de::DeserializeOwned>(raw: &str, source: &str) -> Result<Vec<T>> {
    let mut records = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: T =
            serde_json::from_str(line).with_context(|| format!("{source} line {}", lineno + 1))?;
        records.push(record);
    }
    Ok(records)
}

/// Loads a committed JSONL corpus file (strict per line); `kind` names
/// the file in the read error ("manifest"/"golden" — the exact strings
/// the corpora have always printed).
pub fn load_jsonl<T: serde::de::DeserializeOwned>(path: &Path, kind: &str) -> Result<Vec<T>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading {kind} {}", path.display()))?;
    parse_jsonl(&raw, &path.display().to_string())
}

// ---------------------------------------------------------------------------
// Positional alignment
// ---------------------------------------------------------------------------

/// Position-by-position manifest/corpus alignment — equal line counts
/// and identical id sequences (the truncated/reordered-golden guard,
/// the dsn corpus lesson, shared by `dsn|index|undo compare` and their
/// pins). POSITIONAL, never set equality: a same-count wrong-order
/// corpus must fail here naming the first misaligned line.
///
/// # Argument order is semantic (T1 review note for T15/T16 authors)
///
/// `entries` is the MANIFEST side and `records` the GOLDEN side. Both
/// are `HasId`, so the compiler will NOT catch a swapped call
/// `(…, &golden, …, &manifest)` — it type-checks either way and merely
/// inverts the diagnostic. The error labels are also HARD-CODED
/// ("golden … has N line(s) but manifest …", "golden line N carries id
/// … but manifest line N is …"): pass manifest-entries and
/// golden-records in exactly this order, even when your corpus names
/// the second file something other than a golden.
pub fn ensure_alignment<M: HasId, R: HasId>(
    manifest_path: &Path,
    entries: &[M],
    golden_path: &Path,
    records: &[R],
) -> Result<()> {
    // Counts checked (M1a lesson): a truncated golden must fail loudly,
    // and the id sequences must agree position by position.
    anyhow::ensure!(
        records.len() == entries.len(),
        "golden {} has {} line(s) but manifest {} has {} — recapture needed",
        golden_path.display(),
        records.len(),
        manifest_path.display(),
        entries.len()
    );
    for (index, (entry, record)) in entries.iter().zip(records).enumerate() {
        anyhow::ensure!(
            record.id() == entry.id(),
            "golden line {} carries id {} but manifest line {} is {} — recapture needed",
            index + 1,
            record.id(),
            index + 1,
            entry.id()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Diff rendering (golden-vs-rust value display on a mismatch)
// ---------------------------------------------------------------------------

/// The JSON of a diffed value (the golden-vs-rust display); a
/// serialization failure renders as a placeholder instead of panicking
/// mid-printout.
pub fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string())
}

/// Caps one diff-rendered value at 160 BYTES (byte length, matching the
/// originals) with a char-boundary-safe cut and a `...` tail.
///
/// Why 160: the M2 shell review found this helper TRIPLECTED with a
/// cap drift — dsn_corpus cut at 120 (the M1b original), index/undo at
/// 160 (2-of-3 majority). Unified on 160. The resolved drift touches
/// DIAGNOSTIC TEXT ONLY: every `truncate` call site (dsn/index/undo
/// `compare`) renders golden-vs-rust values on a mismatch — never
/// manifest bytes, never golden bytes, nothing committed or compared.
pub fn truncate(text: &str) -> String {
    const CAP: usize = 160;
    if text.len() <= CAP {
        text.to_string()
    } else {
        let mut cut = CAP;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}...", &text[..cut])
    }
}

// ---------------------------------------------------------------------------
// Pins (fast, synthetic — no filesystem, no JVM)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    /// A minimal corpus record standing in for the per-corpus golden
    /// types (the real ones stay with their corpora — these pins
    /// exercise only the shared mechanics).
    struct Row {
        id: String,
    }

    impl HasId for Row {
        fn id(&self) -> &str {
            &self.id
        }
    }

    fn entries(count: usize) -> Vec<ManifestEntry> {
        (1..=count)
            .map(|no| ManifestEntry {
                id: format!("dsn-{no:04}"),
                path: format!("fixtures/f{no}.dsn"),
            })
            .collect()
    }

    fn rows(ids: &[&str]) -> Vec<Row> {
        ids.iter()
            .map(|id| Row {
                id: (*id).to_string(),
            })
            .collect()
    }

    /// `ensure_alignment` catches a COUNT mismatch in BOTH directions —
    /// more corpus lines than manifest entries AND vice versa (the M1a
    /// truncated-golden lesson; neither may pass silently).
    #[test]
    fn alignment_rejects_count_mismatch_in_both_directions() {
        let manifest = entries(3);
        // More corpus lines than manifest entries.
        let long = rows(&["dsn-0001", "dsn-0002", "dsn-0003", "dsn-0004"]);
        assert!(
            ensure_alignment(Path::new("m.jsonl"), &manifest, Path::new("c.jsonl"), &long).is_err(),
            "an extra corpus line must not align silently"
        );
        // Fewer corpus lines than manifest entries.
        let short = rows(&["dsn-0001", "dsn-0002"]);
        assert!(
            ensure_alignment(
                Path::new("m.jsonl"),
                &manifest,
                Path::new("c.jsonl"),
                &short
            )
            .is_err(),
            "a truncated corpus must not align silently"
        );
    }

    /// Alignment is POSITIONAL, not set equality: the same count with
    /// one swapped id pair must fail, naming the first misaligned line.
    /// (A sorted/set comparison would pass here — that is exactly the
    /// mutant this pin was verified against.)
    #[test]
    fn alignment_is_positional_and_rejects_a_swapped_id() {
        let manifest = entries(3);
        let swapped = rows(&["dsn-0002", "dsn-0001", "dsn-0003"]);
        let Err(err) = ensure_alignment(
            Path::new("m.jsonl"),
            &manifest,
            Path::new("c.jsonl"),
            &swapped,
        ) else {
            panic!("a same-count id swap must not align silently");
        };
        let message = err.to_string();
        assert!(
            message.contains("line 1") && message.contains("dsn-0002"),
            "the error must name the first misaligned position: {message}"
        );
        // The aligned control passes.
        let straight = rows(&["dsn-0001", "dsn-0002", "dsn-0003"]);
        assert!(
            ensure_alignment(
                Path::new("m.jsonl"),
                &manifest,
                Path::new("c.jsonl"),
                &straight
            )
            .is_ok(),
            "the aligned control must pass"
        );
    }

    /// The committed byte format: one COMPACT JSON object per line,
    /// `\n`-terminated including the last, field order id-then-path —
    /// the byte-identity bar the corpora pin their committed manifests
    /// against.
    #[test]
    fn manifest_bytes_is_compact_jsonl_with_trailing_newline() {
        let bytes = manifest_bytes(&entries(2));
        assert_eq!(
            String::from_utf8(bytes).expect("manifest bytes are utf-8"),
            "{\"id\":\"dsn-0001\",\"path\":\"fixtures/f1.dsn\"}\n\
             {\"id\":\"dsn-0002\",\"path\":\"fixtures/f2.dsn\"}\n"
        );
    }

    /// Strict JSONL: blank lines are skipped, a malformed line fails
    /// loudly naming its RAW line number, `deny_unknown_fields` rejects
    /// an extra field, and parse → manifest_bytes round-trips the
    /// committed format.
    ///
    /// Line-numbering convention (T1 review): the malformed `bogus`
    /// sits BEHIND a blank line — raw line 3, but only the 2nd
    /// non-blank line — so the assertion discriminates the committed
    /// raw-lines-enumerate convention from a filter-then-enumerate
    /// refactor (which would print `line 2`). That mutant was applied
    /// and this pin failed against it before landing.
    #[test]
    fn jsonl_loading_is_strict_per_line() {
        let raw = "{\"id\":\"a\",\"path\":\"p\"}\n\n{\"id\":\"b\",\"path\":\"q\"}\n";
        let parsed: Vec<ManifestEntry> = parse_jsonl(raw, "t.jsonl").expect("parses");
        assert_eq!(parsed.len(), 2, "the blank line is skipped");
        let Err(err) =
            parse_jsonl::<ManifestEntry>("{\"id\":\"a\",\"path\":\"p\"}\n\nbogus\n", "t.jsonl")
        else {
            panic!("a malformed line must fail loudly");
        };
        let message = err.to_string();
        assert!(
            message.contains("t.jsonl line 3"),
            "the error must name the offending RAW line (blank lines count): {message}"
        );
        // `deny_unknown_fields` is load-bearing (module + struct docs):
        // a committed line carrying an extra field must be rejected, not
        // silently parsed with the field dropped (the parse path
        // `load_jsonl` delegates to).
        let Err(err) = parse_jsonl::<ManifestEntry>(
            "{\"id\":\"a\",\"path\":\"p\",\"set\":\"x\"}\n",
            "t.jsonl",
        ) else {
            panic!("an unknown field must be rejected, not dropped");
        };
        assert!(
            format!("{err:#}").contains("unknown field"),
            "the rejection must name the unknown-field cause: {err:#}"
        );
        assert_eq!(
            manifest_bytes(&parsed),
            "{\"id\":\"a\",\"path\":\"p\"}\n{\"id\":\"b\",\"path\":\"q\"}\n".as_bytes(),
            "parse → serialize round-trips the committed format (blank lines stripped)"
        );
    }

    /// The unified diff cap is 160 bytes (index/undo's value; dsn's 120
    /// predates it — the drift this extraction resolves). Long ASCII
    /// cuts at exactly 160 with the ellipsis; a multibyte cut retreats
    /// to a char boundary rather than panicking.
    #[test]
    fn truncate_caps_diff_text_at_160_bytes_on_a_char_boundary() {
        let at_cap = "x".repeat(160);
        assert_eq!(truncate(&at_cap), at_cap, "exactly at the cap: unchanged");
        let over_cap = "x".repeat(161);
        assert_eq!(
            truncate(&over_cap),
            format!("{at_cap}..."),
            "one over: cut + ellipsis"
        );
        // 60 × '日' = 180 bytes; byte 160 falls mid-char, so the cut
        // retreats to 159 bytes = 53 chars.
        let multibyte = "日".repeat(60);
        assert_eq!(truncate(&multibyte), format!("{}...", "日".repeat(53)));
    }

    /// The one sha spelling matches the known vectors — the hash the
    /// corpora, the SES capture, and the digests all share. `hex` is
    /// its NON-HASHING sibling: hexing a finished digest must be the
    /// identity on those bytes (the double-hash mutant that briefly
    /// diverged all 33 index fixtures is exactly this confusion).
    #[test]
    fn sha256_hex_matches_known_vectors_and_hex_does_not_rehash() {
        const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(b""), EMPTY_SHA);
        assert_eq!(sha256_hex(b"abc"), ABC_SHA);
        // The index corpus's incremental-dump digest:
        // hex(finish(sha256)) == sha256_hex, byte for byte.
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"abc");
        assert_eq!(hex(&hasher.finalize()), ABC_SHA);
    }
}
