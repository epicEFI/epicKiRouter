//! Upstream #152 (PR #889, d9694ab82): the `planeNets` override.
//!
//! Java `HeadlessBoardManager.applyPlaneNetsOverride`: for every name
//! in the settings list (blank/null entries skipped), resolve the
//! board nets and set `containsPlane = true` on each match that does
//! not already carry it, logging one INFO per flip ("Configured net
//! 'X' as a power plane net via router.plane_nets"). The flag is the
//! router's existing plane switch — plane via costs, plane-connected
//! completion gating — so the override changes routing with zero
//! router edits, the same lever F3's synthesized pours pull.
//!
//! Name resolution: Java `Nets.get(name.trim())` is a
//! case-insensitive ALL-matches collection query. The port keeps the
//! trim and the all-matches shape but goes EXACT-first (the
//! [`crate::pour::resolve_layer`] law: a wrong-case name is a typo,
//! not a different net) — `GND` on a board holding both `GND` and
//! `gnd` flips only the exact one, where Java would flip both; the
//! case-insensitive fallback still catches every typo. Notes are
//! returned (the caller warns through the sink), never printed here
//! — default runs (no flag set) never call this and stay
//! byte-stable.

use epic_board::board::Board;

/// Apply the `router.plane.nets` override: set `contains_plane` on
/// every resolved net that lacks it. Returns the per-flip notes (one
/// per net actually promoted, canonical spelling) plus one
/// not-found note per unresolved name — the caller decides which
/// face each kind reaches (the CLI warns both; the manifest can
/// carry only the rows).
pub fn apply_plane_nets(board: &mut Board, names: &[String]) -> Vec<String> {
    let mut notes = Vec::new();
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Exact matches first; only an exact-empty result falls to
        // the case-insensitive sweep (all matches either way — the
        // Java Collection shape).
        let mut matches: Vec<i32> = board
            .rules()
            .nets
            .iter()
            .filter(|&(_, net)| net.name == trimmed)
            .map(|(net_number, _)| net_number)
            .collect();
        if matches.is_empty() {
            matches = board
                .rules()
                .nets
                .iter()
                .filter(|&(_, net)| net.name.eq_ignore_ascii_case(trimmed))
                .map(|(net_number, _)| net_number)
                .collect();
        }
        if matches.is_empty() {
            notes.push(format!("plane net '{trimmed}' not found on the board"));
            continue;
        }
        for net_number in matches {
            if !board
                .rules()
                .nets
                .get(net_number)
                .is_some_and(|net| net.contains_plane)
            {
                board.rules_mut().nets.set_contains_plane(net_number, true);
                let canonical = board
                    .rules()
                    .nets
                    .get(net_number)
                    .map(|net| net.name.clone())
                    .unwrap_or_else(|| trimmed.to_string());
                notes.push(format!(
                    "net '{canonical}' configured for plane routing via router.plane.nets"
                ));
            }
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    const CRAFT_DSN: &str = r#"(pcb plane-nets-craft.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (path pcb 0  0 0  10000 0  10000 8000  0 8000  0 0))
  )
  (network
    (net GND)
    (net gnd)
    (net PWR)
    (net N1)
  )
  (wiring
    (wire (path F.Cu 125  1000 1000  3000 1000) (net GND))
  )
)
"#;

    fn craft_board() -> Board {
        let mut ses = SesBoard::new();
        match read_board(CRAFT_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    fn plane_flags(board: &Board) -> Vec<(&str, bool)> {
        board
            .rules()
            .nets
            .iter()
            .map(|(_, net)| (net.name.as_str(), net.contains_plane))
            .collect()
    }

    /// The exact-then-ci law, the all-matches ci fallback, the
    /// not-found note, and the idempotent re-apply (an
    /// already-plane net produces NO second note — Java logs the
    /// flip only).
    #[test]
    fn exact_first_then_ci_and_notes() {
        let mut board = craft_board();
        assert_eq!(
            plane_flags(&board),
            vec![
                ("GND", false),
                ("gnd", false),
                ("PWR", false),
                ("N1", false)
            ]
        );

        // Exact: only the exact-case net flips.
        let notes = apply_plane_nets(&mut board, &["GND".to_string()]);
        assert_eq!(
            notes,
            vec!["net 'GND' configured for plane routing via router.plane.nets"]
        );
        assert_eq!(
            plane_flags(&board),
            vec![("GND", true), ("gnd", false), ("PWR", false), ("N1", false)],
            "exact match leaves the case-variant twin alone"
        );

        // ci fallback (no exact 'pwr'): all case-insensitive matches.
        let notes = apply_plane_nets(&mut board, &["pwr".to_string()]);
        assert_eq!(notes.len(), 1, "PWR flips through the fallback");
        assert_eq!(plane_flags(&board)[2], ("PWR", true));

        // Re-apply is note-silent (already plane).
        let notes = apply_plane_nets(&mut board, &["GND".to_string(), "PWR".to_string()]);
        assert!(notes.is_empty(), "no flip, no note");

        // Unknown name -> the not-found note.
        let notes = apply_plane_nets(&mut board, &["  ".to_string(), "NOPE".to_string()]);
        assert_eq!(notes, vec!["plane net 'NOPE' not found on the board"]);
    }
}
