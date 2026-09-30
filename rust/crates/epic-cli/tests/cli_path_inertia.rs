//! M9-T3 PIN 7 — the CLI-path inertia pin: the CLI never constructs
//! the tee sink (the tee/event layer is host-only — the parity stream
//! is byte-stable STRUCTURALLY, not just by measurement). The
//! include_str self-grep pattern: the CLI's sources are embedded at
//! compile time and grep-asserted here (zero `TeeDriverSink` hits);
//! the needle never appears in THIS file's assertion target set (the
//! src files are the scanned corpus, not this test).

/// The scanned corpus: every epic-cli source file (the crate is
/// main.rs + lib.rs + route.rs since the M9-T1 settings move).
const CLI_SOURCES: [&str; 3] = [
    include_str!("../src/main.rs"),
    include_str!("../src/lib.rs"),
    include_str!("../src/route.rs"),
];

#[test]
fn cli_sources_never_construct_the_tee() {
    // Sanity: the include_str faces actually carried the sources.
    assert!(
        CLI_SOURCES.iter().all(|source| !source.is_empty()),
        "the embedded sources are non-empty"
    );
    for (index, source) in CLI_SOURCES.iter().enumerate() {
        assert!(
            !source.contains(concat!("Tee", "DriverSink")),
            "epic-cli source #{index} references the tee sink (the CLI path must never construct it)"
        );
    }
}
