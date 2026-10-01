//! The `(plane ...)` scope reader: the port of `io/specctra/parser/Plane
//! .java` `readScope` (`:53-83`).
//!
//! Java call shape: dispatched from the structure reader
//! (`Structure.java:1001-1005`) AFTER the layer-structure guard, and the
//! boolean return value is DISCARDED at the dispatch site — a failed read
//! (non-string net name) contributes nothing and does NOT fail the parse.
//! The accumulated [`PlaneInfo`]s are consumed by the structure post-loop
//! (`Structure.java:1067-1122`).

use crate::lexer::{Scanner, Token};
use crate::shape::read_area_scope;
use crate::state::{ParseState, PlaneInfo};

/// Java `Plane.readScope` (`Plane.java:53-83`). The net name must scan as
/// a String token (the NAME lexical state even renders a bare `123` a
/// string — jar `/tmp/epic-t6-board.out` t6-plane-badnet: `(plane 123
/// ...)` succeeds with net "123"); anything else warns "String expected"
/// and returns `false` WITHOUT appending a [`PlaneInfo`].
///
/// Bug-compat note: the [`read_area_scope`] result is stored UNCHECKED
/// (`:79-81`) — a failed area read lands as `None` (Java null) in
/// [`PlaneInfo::area`] and only surfaces as the `Structure.java:1084` NPE
/// at insertion time (parse-fatal parity).
pub(crate) fn read_plane_scope(scanner: &mut Scanner, state: &mut ParseState) -> bool {
    // Cadence Allegro cuts the pins out of power planes; those `(window
    // ...)` scopes are skipped (`:57`, `"allegro".equalsIgnoreCase(hostCad)`)
    let skip_window_scopes = state
        .host_cad
        .as_deref()
        .is_some_and(|cad| cad.eq_ignore_ascii_case("allegro"));
    let Token::Str(net_name) = scanner.next_token() else {
        // Java warns "Plane.read_scope: String expected"
        return false;
    };
    // `scanner.setScopeIdentifier(netName)` (`:71`) is FRLogger-cosmetic.
    let area = read_area_scope(scanner, state.layer_structure.as_ref(), skip_window_scopes);
    state.plane_list.push(PlaneInfo {
        area,
        net_name: net_name.to_string(),
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{LexicalState, Scanner};

    /// Scanner in the state the dispatch site leaves it in: the `plane`
    /// keyword switches the Java scanner to its NAME state
    /// (`Keyword::Plane => LexicalState::Name`), where EVERYTHING —
    /// integers (`(net 5)` -> STR, jar-cited in the lexer) and `signal`
    /// included — scans as a string word.
    fn name_state_scanner(input: &[u8]) -> Scanner<'_> {
        let mut scanner = Scanner::new(input);
        scanner.set_lexical_state(LexicalState::Name);
        scanner
    }

    /// A string net name appends a PlaneInfo even when the area read
    /// fails (Java stores the null area unchecked, `Plane.java:79-81`).
    #[test]
    fn numeric_net_name_is_a_string_and_bad_area_is_stored_unchecked() {
        let mut state = ParseState::default();
        let mut scanner = name_state_scanner(b"123 (rect signal 0 0 10 10))");
        assert!(read_plane_scope(&mut scanner, &mut state));
        assert_eq!(state.plane_list.len(), 1);
        assert_eq!(state.plane_list[0].net_name, "123");
        // the rect read succeeds (signal pseudo-layer, no structure needed)
        assert!(state.plane_list[0].area.is_some());

        // a failed area read still appends (area None = Java null)
        let mut state = ParseState::default();
        let mut scanner = name_state_scanner(b"gnd (nosuchshape x)))");
        assert!(read_plane_scope(&mut scanner, &mut state));
        assert_eq!(state.plane_list.len(), 1);
        assert!(state.plane_list[0].area.is_none());
    }

    /// A non-string net name returns false and appends NOTHING — the
    /// discarded return value means no parse failure. In NAME state only
    /// brackets and EOF are non-strings (integers/keywords scan as
    /// words), so the bracket is the reachable discriminator.
    #[test]
    fn bracket_net_name_appends_nothing() {
        let mut state = ParseState::default();
        let mut scanner = name_state_scanner(b"(gnd (rect signal 0 0 10 10)))");
        assert!(!read_plane_scope(&mut scanner, &mut state));
        assert!(state.plane_list.is_empty());
    }

    /// `host_cad = "allegro"` (case-insensitive, `:57`) skips `(window
    /// ...)` holes; other hosts keep them.
    #[test]
    fn allegro_host_skips_window_scopes() {
        let mut state = ParseState {
            host_cad: Some("Allegro".to_string()),
            ..Default::default()
        };
        let mut scanner =
            name_state_scanner(b"gnd (rect signal 0 0 10 10) (window (rect signal 1 1 2 2)))");
        assert!(read_plane_scope(&mut scanner, &mut state));
        let area = state.plane_list[0].area.as_ref().expect("area");
        assert_eq!(area.shapes.len(), 1, "window skipped");

        let mut state = ParseState {
            host_cad: Some("KiCad".to_string()),
            ..Default::default()
        };
        let mut scanner =
            name_state_scanner(b"gnd (rect signal 0 0 10 10) (window (rect signal 1 1 2 2)))");
        assert!(read_plane_scope(&mut scanner, &mut state));
        let area = state.plane_list[0].area.as_ref().expect("area");
        assert_eq!(area.shapes.len(), 2, "window kept as a hole");
    }
}
