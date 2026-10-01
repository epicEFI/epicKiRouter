//! THE workflow tripwire pin (M3-T17c; closes banked mutant S6): the CI
//! workflow `.github/workflows/rust-check.yml` is load-bearing policy —
//! the test step's 30-minute wall (the hang tripwire the bare suite
//! leans on since the T17b skip retirement) and the router-compare
//! step's argv are both SINGLE LINES whose silent mutation would
//! change CI's contract with nothing in the suite noticing. This module
//! pins both faces: a tiny step-block reader for the workflow's shape
//! (pure, synthetic-testable — not a YAML parser, by design) and the
//! real-file pin. THE M8-T8 EXIT FLIP HAS LANDED (commit `ba5e587bb`):
//! the compare step rides the exact GATE argv
//! (`cargo run -q -p epic-harness -- router compare` — no
//! `--report-only`; exit is a hard gate), and the events-compare step
//! (added by the same flip commit) is pinned with unique-existence +
//! its own wall bound. The pin now DIES on a silent re-ADDITION of
//! `--report-only`, any argv drift on the three pinned steps, or any
//! wall move — the change must be a conscious same-commit act that
//! updates the pin (the OBS-1 precedent: policy edits and their pins
//! land together).

// ---------------------------------------------------------------------------
// The workflow reader (pure; synthetic-world pinnable)
// ---------------------------------------------------------------------------

/// Splits the workflow YAML into its STEP blocks. A step block starts at
/// a line whose (stripped) text starts with `- ` at the FIRST step's
/// indent (the file's step level) and runs to the line before the next
/// such line; interleaved comment/blank lines at any indent attach to
/// the PRECEDING block. Deliberately NOT a YAML parser: the pin needs
/// the file's step grouping (a `timeout-minutes:` belongs to ITS step),
/// which line ranges express exactly; anything richer drifts with the
/// yaml crate.
pub fn workflow_step_blocks(raw: &str) -> Vec<String> {
    let lines: Vec<&str> = raw.lines().collect();
    let step_indent = lines
        .iter()
        .filter_map(|line| {
            let stripped = line.trim_start();
            stripped
                .starts_with("- ")
                .then(|| line.len() - stripped.len())
        })
        .next();
    let Some(step_indent) = step_indent else {
        return Vec::new();
    };
    let mut blocks: Vec<String> = Vec::new();
    for line in lines {
        let stripped = line.trim_start();
        let is_step_head = stripped.starts_with("- ") && line.len() - stripped.len() == step_indent;
        if is_step_head {
            blocks.push(line.to_string());
        } else if let Some(block) = blocks.last_mut() {
            block.push('\n');
            block.push_str(line);
        }
    }
    blocks
}

/// Every `run:` line containing `needle` across ALL blocks — the
/// diagnostic twin of [`find_step_run_line`], so a trip can name the
/// actual condition: zero matches (the step is gone) vs N matches (a
/// duplicated step is the usual cause).
fn matching_run_lines<'a>(blocks: &'a [String], needle: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    for block in blocks {
        for line in block.lines() {
            let stripped = line.trim_start();
            let stripped = stripped.strip_prefix("- ").unwrap_or(stripped);
            if stripped.starts_with("run:") && stripped.contains(needle) {
                found.push(line);
            }
        }
    }
    found
}

/// The ONE `run:` line containing `needle` across ALL blocks — the run
/// line, not the block text: comment lines routinely mention step names
/// (`# M3 T15 router compare: ...` precedes the compare step) and a
/// block-text search would anchor on them. Returns `None` when zero OR
/// MULTIPLE run lines match (a duplicated step must trip the pin, not
/// silently pick one).
#[must_use]
pub fn find_step_run_line<'a>(blocks: &'a [String], needle: &str) -> Option<&'a str> {
    let mut found = matching_run_lines(blocks, needle);
    match found.len() {
        1 => found.pop(),
        _ => None, // zero (missing) or many (duplicate): trip, never guess
    }
}

/// The step's OWN `timeout-minutes:` value — a line inside `block`
/// whose stripped text is `timeout-minutes: <int>` at deeper indent
/// than the step head (the step-scoped property, not a comment mention
/// and not another step's). Zero such lines or a non-integer value →
/// `None`; more than one → `None` (ambiguous). A trailing INLINE
/// comment is tolerated (`timeout-minutes: 30 # note` — cosmetics, not
/// a contract change), but per the YAML plain-scalar rule a `#` starts
/// a comment only after whitespace or at the value's start: `3#0` is
/// the non-integer scalar "3#0" and trips; a comment in the VALUE
/// position (`timeout-minutes: # 30`) leaves the value empty and also
/// trips.
#[must_use]
pub fn step_timeout_minutes(block: &str) -> Option<u64> {
    let mut found: Option<u64> = None;
    for line in block.lines().skip(1) {
        let stripped = line.trim();
        let Some(rest) = stripped.strip_prefix("timeout-minutes:") else {
            continue;
        };
        // Strip a trailing inline comment BEFORE the digit test — but
        // only a WHITESPACE-preceded `#` is a comment (YAML rule):
        // `30  # hang tripwire` reads 30, while `3#0` keeps its `#` and
        // trips the digit test below (never guess a partial value).
        let value = match rest.split_once(" #") {
            Some((head, _comment)) => head.trim(),
            None => rest.trim(),
        };
        if found.is_some() || value.is_empty() || !value.chars().all(|c| c.is_ascii_digit()) {
            return None; // duplicate or non-integer: trip, never guess
        }
        found = value.parse().ok();
    }
    found
}

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use super::*;

    /// The reader's discriminating faces on a synthetic workflow:
    /// (i) blocks isolate steps so a step's `timeout-minutes` never
    /// leaks into the neighbor (the 5 vs 30 pair); (ii) a step without
    /// the property reads `None` (the missing-arm face — a parser that
    /// defaulted to Some(anything) would hide a dropped wall);
    /// (iii) the COMMENT-TRAP: an earlier step's comment mentioning
    /// `router compare` cannot steal the anchor — the finder returns
    /// the LATER genuine run line (kills the block-text-search mutant);
    /// (iv) two steps matching the same needle read ambiguous `None`.
    #[test]
    fn workflow_reader_reads_blocks_run_lines_and_own_timeouts() {
        let raw = "\
jobs:
  check:
    steps:
      - run: cargo fmt --all --check
      # a comment mentioning router compare --report-only (the trap)
      - run: cargo test --workspace
        timeout-minutes: 30
      - run: cargo build -q -p epic-cli
        timeout-minutes: 5
      - run: cargo run -q -p epic-harness -- router compare --report-only
        timeout-minutes: 30
        env:
          EPIC_SKIP_GRADLE: \"1\"
";
        let blocks = workflow_step_blocks(raw);
        assert_eq!(blocks.len(), 4, "four steps: {blocks:?}");

        // (i) each step reads ITS OWN timeout (no leak between blocks).
        let test = blocks
            .iter()
            .find(|b| b.contains("cargo test --workspace"))
            .expect("test step");
        assert_eq!(step_timeout_minutes(test), Some(30));
        let build = blocks
            .iter()
            .find(|b| b.contains("epic-cli"))
            .expect("build step");
        assert_eq!(step_timeout_minutes(build), Some(5));

        // (ii) the missing arm: no property → None (never a default).
        let fmt = blocks.first().expect("fmt step");
        assert_eq!(step_timeout_minutes(fmt), None);

        // (iii) THE comment-trap: the needle search must anchor on the
        // run line of the LATER step, not the earlier comment.
        let found_line = find_step_run_line(&blocks, "router compare")
            .expect("the genuine compare run line, past the trap comment");
        assert!(
            found_line.contains("cargo run"),
            "the finder must return the RUN line, not a comment: {found_line}"
        );
        assert!(
            found_line.contains("--report-only"),
            "the genuine line carries the flag: {found_line}"
        );

        // (iv) ambiguity: a second step with the same needle → None.
        let mut ambiguous = blocks.clone();
        ambiguous.push("      - run: cargo test --workspace\n".into());
        assert_eq!(
            find_step_run_line(&ambiguous, "cargo test --workspace"),
            None,
            "two matching run lines are ambiguous and must trip"
        );

        // The non-integer face: a wall typo reads None, never a guess.
        let typoed = "      - run: x\n        timeout-minutes: soon\n".to_string();
        assert_eq!(step_timeout_minutes(&typoed), None);
        // The duplicate face: two walls in one step are ambiguous.
        let doubled =
            "      - run: x\n        timeout-minutes: 30\n        timeout-minutes: 5\n".to_string();
        assert_eq!(step_timeout_minutes(&doubled), None);
        // The TOLERANT face (T17c quality MINOR-3): a trailing inline
        // comment is cosmetics, not a contract change — the value reads.
        let commented = "      - run: x\n        timeout-minutes: 30  # hang tripwire\n";
        assert_eq!(step_timeout_minutes(commented), Some(30));
        // But a comment in the VALUE position leaves nothing to read —
        // it must still trip (the comment-before-value face).
        let value_comment = "      - run: x\n        timeout-minutes: # 30\n";
        assert_eq!(step_timeout_minutes(value_comment), None);
        // And a `#` NOT preceded by whitespace is part of the plain
        // scalar (YAML rule): `3#0` is no integer — trip, never guess
        // (re-review Q8's residual; the old strip guessed Some(3)).
        let glued = "      - run: x\n        timeout-minutes: 3#0\n";
        assert_eq!(step_timeout_minutes(glued), None);
    }

    /// THE tripwire, post-retirement (real repo path): GitHub CI is
    /// RETIRED for the standalone-repo era (2026-10-01, 8e018cfa5 —
    /// all workflow files + dependabot removed; Tyler: runs quiet
    /// "for now"). The retired file must STAY gone: the 2-core hosted
    /// runner cannot fit the corpus-compare step (bm01 alone = 1770s
    /// vs the 30-minute step budget; it failed on timeout with bm01
    /// PASSING). Restoring CI is a conscious act that must first split
    /// the compare step or raise its wall — then this pin returns to
    /// its historical form (the pre-retirement faces: the bare-suite
    /// argv + 30-min test wall, the post-M8-T8-flip gate argv, the
    /// events step's 10-min wall, the M9-T6 desktop-clippy and
    /// threads-invariance steps — the exact assertions live in git
    /// history at 8e018cfa5^).
    #[test]
    fn ci_workflow_is_retired_and_stays_gone() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let path = root.join(".github/workflows/rust-check.yml");
        assert!(
            !path.exists(),
            "rust-check.yml is retired (8e018cfa5): the 2-core runner blew the \
             compare step's 30-minute budget at bm01=1770s. To restore CI, split or \
             re-budget the compare step FIRST, then re-pin the workflow here: {}",
            path.display()
        );
    }

    #[allow(dead_code)] // the pre-retirement pin body, kept for the
    // restoration: re-attach to a #[test] when CI returns (see the
    // absence pin above for the re-entry protocol).
    fn ci_workflow_pins_test_step_wall_and_compare_gate_argv() {
        let root = crate::oracle::find_repo_root().expect("repo root");
        let path = root.join(".github/workflows/rust-check.yml");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("workflow readable at {}: {e}", path.display()));
        let blocks = workflow_step_blocks(&raw);

        // THE TEST STEP: exact bare argv + its own 30-minute wall.
        let test_line =
            find_step_run_line(&blocks, "cargo test --workspace").unwrap_or_else(|| {
                let hits = matching_run_lines(&blocks, "cargo test --workspace");
                panic!(
                    "the bare-suite run line must exist UNIQUELY — {} matching run \
                     line(s) (zero = the step is gone or its run line was folded/reshaped; multiple = a duplicated step \
                     is the usual cause): {hits:?}",
                    hits.len()
                );
            });
        let test_argv = test_line
            .trim_start()
            .strip_prefix("- ")
            .unwrap_or(test_line.trim_start());
        assert_eq!(
            test_argv, "run: cargo test --workspace",
            "the test step must stay BARE (a --skip filter returning is a conscious act): {test_line}"
        );
        let test_block = blocks
            .iter()
            .find(|b| b.lines().any(|l| l == test_line))
            .expect("the test step's own block");
        assert_eq!(
            step_timeout_minutes(test_block),
            Some(30),
            "the test step's 30-minute wall is the hang tripwire: {}",
            test_block
        );

        // THE COMPARE STEP (post-flip): the EXACT gate argv on the run
        // line — `--report-only` must stay gone (the flip's other
        // direction: silently re-adding the flag would silent the hard
        // gate), and the designed battery-wall tripwire stays 30.
        let compare_line = find_step_run_line(&blocks, "router compare").unwrap_or_else(|| {
            let hits = matching_run_lines(&blocks, "router compare");
            panic!(
                "the router-compare run line must exist UNIQUELY — {} matching run \
                 line(s) (zero = the step is gone or its run line was folded/reshaped; multiple = a duplicated step is \
                 the usual cause): {hits:?}",
                hits.len()
            );
        });
        let compare_argv = compare_line
            .trim_start()
            .strip_prefix("- ")
            .unwrap_or(compare_line.trim_start());
        assert_eq!(
            compare_argv, "run: cargo run -q -p epic-harness -- router compare",
            "the compare step must ride the exact GATE argv (the M8-T8 exit flip; \
             re-adding --report-only is a conscious act that must die here): {compare_line}"
        );
        let compare_block = blocks
            .iter()
            .find(|b| b.lines().any(|l| l == compare_line))
            .expect("the compare step's own block");
        assert_eq!(
            step_timeout_minutes(compare_block),
            Some(30),
            "the compare step's 30-minute wall is the designed wall tripwire: {compare_block}"
        );

        // THE EVENTS STEP (the flip commit's addition): exists UNIQUELY
        // with its own wall bound — the java-free events-compare gate
        // (3 fixtures / 3,520 golden trace rows, green since M4-T6).
        let events_line = find_step_run_line(&blocks, "events compare").unwrap_or_else(|| {
            let hits = matching_run_lines(&blocks, "events compare");
            panic!(
                "the events-compare run line must exist UNIQUELY — {} matching run \
                 line(s) (zero = the step is gone or its run line was folded/reshaped; multiple = a duplicated step is \
                 the usual cause): {hits:?}",
                hits.len()
            );
        });
        let events_argv = events_line
            .trim_start()
            .strip_prefix("- ")
            .unwrap_or(events_line.trim_start());
        assert_eq!(
            events_argv, "run: cargo run -q -p epic-harness -- events compare",
            "the events step must ride its exact argv: {events_line}"
        );
        let events_block = blocks
            .iter()
            .find(|b| b.lines().any(|l| l == events_line))
            .expect("the events step's own block");
        assert_eq!(
            step_timeout_minutes(events_block),
            Some(10),
            "the events step carries its own 10-minute wall bound: {events_block}"
        );

        // THE DESKTOP-CLIPPY STEP (M9-T6): the default-off `desktop`
        // feature's ONLY CI face — exact argv + unique existence +
        // its own 30-minute wall (the battery-precedent bound; the
        // wgpu tree compiles on a 2-core runner).
        let desktop_line =
            find_step_run_line(&blocks, "--features desktop").unwrap_or_else(|| {
                let hits = matching_run_lines(&blocks, "--features desktop");
                panic!(
                    "the desktop-clippy run line must exist UNIQUELY — {} matching run \
                     line(s) (zero = the step is gone or its run line was folded/reshaped; multiple = a duplicated step is \
                     the usual cause): {hits:?}",
                    hits.len()
                );
            });
        let desktop_argv = desktop_line
            .trim_start()
            .strip_prefix("- ")
            .unwrap_or(desktop_line.trim_start());
        assert_eq!(
            desktop_argv,
            "run: cargo clippy -p epic-gui --all-targets --features desktop -- -D warnings",
            "the desktop-clippy step must ride its exact argv (a gate-shape change \
             is a conscious act that must update this pin): {desktop_line}"
        );
        let desktop_block = blocks
            .iter()
            .find(|b| b.lines().any(|l| l == desktop_line))
            .expect("the desktop-clippy step's own block");
        assert_eq!(
            step_timeout_minutes(desktop_block),
            Some(30),
            "the desktop-clippy step carries its own 30-minute wall (the wgpu tree \
             compiles on a 2-core runner): {desktop_block}"
        );

        // THE THREADS-GATE STEP (M9-T6 — the M5-T8 debt lands): the
        // in-tree threads-invariance harness gate joins CI, exact
        // argv + unique existence + its own 30-minute wall (the
        // determinism-step precedent).
        let threads_line =
            find_step_run_line(&blocks, "router threads-invariance").unwrap_or_else(|| {
                let hits = matching_run_lines(&blocks, "router threads-invariance");
                panic!(
                    "the threads-invariance run line must exist UNIQUELY — {} matching run \
                     line(s) (zero = the step is gone or its run line was folded/reshaped; multiple = a duplicated step is \
                     the usual cause): {hits:?}",
                    hits.len()
                );
            });
        let threads_argv = threads_line
            .trim_start()
            .strip_prefix("- ")
            .unwrap_or(threads_line.trim_start());
        assert_eq!(
            threads_argv,
            "run: cargo run -q -p epic-harness --release -- router threads-invariance",
            "the threads-gate step must ride its exact argv (the M5-T8 promise, \
             landed; a gate-shape change must update this pin): {threads_line}"
        );
        let threads_block = blocks
            .iter()
            .find(|b| b.lines().any(|l| l == threads_line))
            .expect("the threads-gate step's own block");
        assert_eq!(
            step_timeout_minutes(threads_block),
            Some(30),
            "the threads-gate step carries its own 30-minute wall (the determinism- \
             step precedent): {threads_block}"
        );
    }
}
