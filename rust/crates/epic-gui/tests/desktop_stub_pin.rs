//! The M9-T6 stub-bin pin: built WITHOUT the `desktop` feature, the
//! `epic-gui` bin prints the hint and exits 1 (the default
//! workspace build compiles the stub, never the wgpu tree).
//! Freshness: `CARGO_BIN_EXE_epic-gui` is cargo's OWN freshness
//! contract — the env var names the bin cargo built for THIS test
//! run (the DNR-17 spawned-bin discipline, honored structurally).

//! Only meaningful in the default-feature face: with `desktop` on,
//! the bin is the real shell (and would open a window) — cfg-gated
//! out of the desktop-clippy face.

#[cfg(not(feature = "desktop"))]
#[test]
fn stub_bin_prints_hint_and_exits_1() {
    let bin = env!("CARGO_BIN_EXE_epic-gui");
    let output = std::process::Command::new(bin)
        .output()
        .unwrap_or_else(|error| panic!("the stub bin spawns: {error}"));
    let code = output.status.code();
    assert_eq!(code, Some(1), "the stub exits 1: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("desktop") && stderr.contains("--features"),
        "the hint names the feature and the rebuild flag: {stderr}"
    );
}
