//! Port of `io.specctra.DsnReader` (`DsnReader.java`) — the read_board
//! assembly that ties the scope readers together: the 3-token
//! `(pcb <name>` header scan, the pcb-level dispatch loop
//! (`ScopeKeyword.readScope`, `ScopeKeyword.java:46-81`, with the eleven
//! scope-keyword readers substituted for the Java `instanceof`
//! dispatch), and the result classification
//! (`BoardReadResult.java:22-31`).
//!
//! Deliberately deferred (Task 10): `readMetadata` (`DsnReader.java:
//! 182-280`) — its early-stop-on-failure ordering is observable only
//! through the BoardMetadata it builds, which this port materializes in
//! M3. The design name token is inert here (Java uses it for FRLogger
//! text and `effectiveDesignName`); it is parsed for cursor parity only.

use crate::keyword::{Keyword, skip_scope};
use crate::lexer::{LexicalState, Scanner, Token};
use crate::scope::{
    library, network, parser_scope, placement, plane, resolution, structure, wiring,
};
use crate::sink::{BoardSink, MetadataIr};
use crate::state::ParseState;

/// Mirror of `BoardReadResult` (`io/BoardReadResult.java:22-31`): the
/// outcome of a full board read. `IoError` exists in the Java sealed
/// hierarchy for stream failures; this port reads from in-memory bytes,
/// so it is never constructed — kept for shape parity.
#[derive(Debug, PartialEq)]
pub enum DsnReadResult {
    /// Java `Success(board, warnings)`: the scope walk completed. The
    /// board itself lives on the sink; only the parity warnings surface
    /// here.
    Success {
        /// The D12 parity warnings collected during the read.
        warnings: Vec<String>,
    },
    /// Java `OutlineMissing(warnings)`: the walk failed with
    /// `boardOutlineOk == false` (no usable `(boundary ...)`).
    OutlineMissing {
        /// The D12 parity warnings collected during the read.
        warnings: Vec<String>,
    },
    /// Java `ParseError(location, detail)`: the header or the scope walk
    /// failed otherwise.
    ParseError {
        /// Java `location` — always `"(pcb"` from this reader.
        location: String,
        /// Java `detail`.
        detail: String,
    },
    /// Java `IoError` — never constructed by this port (module docs).
    IoError,
}

/// Java `DsnReader.readBoard` (`:58-161`), minus the metadata build
/// (Task 10) and the stream plumbing (bytes in, sink out).
pub fn read_board(input: &[u8], sink: &mut dyn BoardSink) -> DsnReadResult {
    let mut scanner = Scanner::new(input);
    // The 3-token header hand-scan (`DsnReader.java:83-109`): `(`, the
    // `pcb` keyword, then the design name in the forced NAME state. The
    // name token is inert (module docs) but still consumed.
    let mut pcb_name: Option<String> = None;
    for i in 0..3 {
        let token = scanner.next_token();
        let ok = match i {
            0 => token == Token::Open,
            // Java `:96` sets the NAME state UNCONDITIONALLY inside the
            // ok-branch — `(` + wrong keyword still flips the state before
            // the failure returns.
            1 => {
                let ok = token == Token::Keyword(Keyword::Pcb);
                scanner.set_lexical_state(LexicalState::Name);
                ok
            }
            // `:100-108`: no ok check — EOF passes (a nameless pcb reads
            // on, jar `/tmp/epic-t9-res.out` HEADER hdpcbonly).
            _ => {
                if let Token::Str(name) = token {
                    pcb_name = Some(name.to_string());
                }
                true
            }
        };
        if !ok {
            return DsnReadResult::ParseError {
                location: "(pcb".to_string(),
                detail: "Not a Specctra DSN file: expected '(pcb <name>' header".to_string(),
            };
        }
    }
    let _ = pcb_name;

    let mut state = ParseState::default();
    let read_ok = read_pcb_scope(&mut scanner, &mut state, sink);
    if read_ok {
        // Java's Success carries metadata = null here; the snapshot below
        // is a DIGEST-side necessity — the metadata must include the
        // board-held flip_style (`Structure.java:1041-1043`, Task 6's
        // `set_flip_style`), which the sink merges in `set_metadata`.
        sink.set_metadata(MetadataIr {
            unit: state.unit,
            resolution: state.resolution,
            string_quote: state.string_quote.clone(),
            snap_angle: state.snap_angle,
            flip_style: None,
            host_cad: state.host_cad.clone(),
            host_version: state.host_version.clone(),
        });
        // Java `:134-136`: the plane heuristic fires only when the DSN
        // carried no `(autoroute_settings ...)` scope.
        if state.autoroute_settings.is_none() {
            sink.adjust_plane_autoroute_settings();
        }
        DsnReadResult::Success {
            warnings: state.warnings,
        }
    } else if !state.board_outline_ok {
        DsnReadResult::OutlineMissing {
            warnings: state.warnings,
        }
    } else {
        DsnReadResult::ParseError {
            location: "(pcb".to_string(),
            detail: "DSN structure parsing failed".to_string(),
        }
    }
}

/// Java `ScopeKeyword.readScope` (`ScopeKeyword.java:46-81`) + the
/// `PCB_SCOPE` subclass table (`DsnReader.java:28`): the pcb-level
/// dispatch. `true` on a clean close or end of file, `false` when any
/// child reader fails (the whole read then classifies by
/// `board_outline_ok`).
fn read_pcb_scope(scanner: &mut Scanner, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    // Java inits nextToken = null — `None` models that, `Token::Eof` does
    // not (the first iteration must not take the `prev == Open` arm).
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Error(_)) {
            // Java's nextToken THROWS (integer overflow); the base loop's
            // IOException-shaped catch returns false.
            return false;
        }
        if next_token == Token::Eof {
            // Java `:55-57`: `null` -> end of file -> TRUE (jar HEADER
            // hdpcbonly/hdpcbname: Success).
            return true;
        }
        if next_token == Token::Close {
            // end of scope — checked BEFORE the arm gate (load-bearing
            // for every desync cascade, Wiring/Task 9).
            break;
        }
        let mut read_ok = true;
        if prev_token == Some(Token::Open) {
            read_ok = match next_token {
                Token::Keyword(Keyword::Component) => {
                    placement::read_component_scope(scanner, state, sink)
                }
                Token::Keyword(Keyword::Library) => {
                    library::read_library_scope(scanner, state, sink)
                }
                Token::Keyword(Keyword::Network) => network::read_scope(scanner, state, sink),
                Token::Keyword(Keyword::PartLibrary) => library::read_part_library_scope(
                    scanner,
                    &mut state.logical_part_mappings,
                    &mut state.logical_parts,
                ),
                Token::Keyword(Keyword::Parser) => parser_scope::read_scope(scanner, state),
                Token::Keyword(Keyword::PlaceControl) => {
                    placement::read_place_control_scope(scanner, sink)
                }
                Token::Keyword(Keyword::Placement) => {
                    placement::read_placement_scope(scanner, state, sink)
                }
                Token::Keyword(Keyword::Plane) => plane::read_plane_scope(scanner, state),
                Token::Keyword(Keyword::Resolution) => resolution::read_scope(scanner, state),
                Token::Keyword(Keyword::Structure) => structure::read_scope(scanner, state, sink),
                Token::Keyword(Keyword::Wiring) => wiring::read_scope(scanner, state, sink),
                // `ScopeKeyword.java:74-76`: any other scope is skipped
                // whole — skipScope is called BARE and its result
                // DISCARDED, so a skip failure (EOF inside the unknown
                // scope) is NOT a read failure: the loop continues, the
                // next token is EOF and readScope returns true (`:55-57`;
                // jar /tmp/t9rev-two.out CASE eof).
                _ => {
                    let _ = skip_scope(scanner);
                    true
                }
            };
        }
        if !read_ok {
            return false;
        }
        prev_token = Some(next_token);
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::reader::read_board;
    use crate::ses_board::{ItemIr, SesBoard};
    use crate::state::{AngleRestriction, Unit};

    // ==== Fixtures (exact bytes of /tmp/epic-t9-*.dsn; every pin below
    // was captured fresh from the frozen jar,
    // /tmp/epic-t9-{res,om section of fresh,fresh3}.out). ====

    const HEADER_GARBAGE_DSN: &str = "garbage";
    const HEADER_BARE_PCB_DSN: &str = "(pcb";
    const HEADER_PCB_NAME_DSN: &str = "(pcb x";
    const HEADER_PCB_EXTRA_DSN: &str = "(pcb x (resolution um 10))";
    const OM_DSN: &str = r#"(pcb t9-om.dsn
  (structure
    (layer F.Cu (type signal))
  )
  (resolution micron 10)
)
"#;
    const PARSERX_DSN: &str = r#"(pcb t9-parserx.dsn
  (parser
    (string_quote ')
    (host_cad KICAD)
    (host_version 7.99)
    (constant abc 5)
    (constant foo bar)
    (write_resolution um 10)
    (generated_by_freerouting)
    (space_in_quoted_tokens on)
  )
  (resolution mm 7)
  (unit mm)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
)
"#;

    /// The t37 plane-heuristic family. FIRE variants carry the one-line
    /// KiCad `(autoroute_settings ...)` scope at PCB level (skip-scoped —
    /// the heuristic gate stays open); GATE variants carry the multi-line
    /// scope INSIDE `(structure ...)` (parsed — the gate closes). The
    /// fresh jar pins live in /tmp/epic-t9-fresh.out CASES t37fire,
    /// t37nofire, t37fire2, t37nofire2, t37gated, t37gate2, t37fire2gate.
    const T37FIRE_DSN: &str = r#"(pcb t9-t37fire.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (plane GND
      (polygon In1.Cu 0  1000 1000  9000 1000  9000 9000  1000 9000)
    )
  )
  (network
    (net GND)
  )
  (wiring
    (wire (path F.Cu 125  2000 1000  3000 1000) (net GND))
  )
)
"#;
    const T37NOFIRE_DSN: &str = r#"(pcb t9-t37nofire.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (plane GND
      (polygon In1.Cu 0  1000 1000  9000 1000  9000 9000  1000 9000)
    )
  )
  (network
    (net GND)
  )
  (wiring
    (wire (path F.Cu 125  2000 1000  3000 1000) (net GND))
    (wire (path In1.Cu 125  2000 2000  3000 2000) (net GND))
  )
)
"#;
    const T37FIRE2_DSN: &str = r#"(pcb t9-t37fire2.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net GND)
  )
  (wiring
    (wire (rectangle In1.Cu 1000 1000 9000 9000) (net GND))
  )
)
"#;
    const T37NOFIRE2_DSN: &str = r#"(pcb t9-t37nofire2.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (network
    (net GND)
  )
  (wiring
    (wire (rectangle In1.Cu 1000 1000 9000 9000) (net GND))
    (wire (path In1.Cu 125  9100 1000  9800 1000) (net GND))
  )
)
"#;
    const T37GATED_DSN: &str = r#"(pcb t9-t37fire.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (plane GND
      (polygon In1.Cu 0  1000 1000  9000 1000  9000 9000  1000 9000)
    )
  )
  (autoroute_settings (run_router on) (run_optimizer off) (vias_count 4) (via_costs 1) (plane_via_costs 1) (start_ripup_costs 1) (layer_rule F.Cu (active on) (preferred_direction off) (costs 1)) (layer_rule In1.Cu (active on) (preferred_direction off) (costs 1)) (layer_rule In2.Cu (active on) (preferred_direction off) (costs 1)) (layer_rule B.Cu (active on) (preferred_direction off) (costs 1)))
  (network
    (net GND)
  )
  (wiring
    (wire (path F.Cu 125  2000 1000  3000 1000) (net GND))
  )
)
"#;
    const T37GATE2_DSN: &str = r#"(pcb t9-t37fire2.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (autoroute_settings (run_router on) (run_optimizer off) (vias_count 4) (via_costs 1) (plane_via_costs 1) (start_ripup_costs 1) (layer_rule F.Cu (active on) (preferred_direction off) (costs 1)) (layer_rule In1.Cu (active on) (preferred_direction off) (costs 1)) (layer_rule In2.Cu (active on) (preferred_direction off) (costs 1)) (layer_rule B.Cu (active on) (preferred_direction off) (costs 1)))
  (network
    (net GND)
  )
  (wiring
    (wire (rectangle In1.Cu 1000 1000 9000 9000) (net GND))
  )
)
"#;
    const T37FIRE2GATE_DSN: &str = r#"(pcb t9-t37fire2.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer In2.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (autoroute_settings
      (fanout off)
      (autoroute on)
      (postroute off)
      (vias off)
      (via_costs 7)
      (plane_via_costs 0)
      (start_ripup_costs -5)
      (layer_rule F.Cu
        (active on)
        (preferred_direction horizontal)
      )
      (layer_rule In1.Cu
        (active off)
        (preferred_direction vertical)
        (preferred_direction_trace_costs 2.5)
        (against_preferred_direction_trace_costs 0.5)
      )
    )
  )
  (network
    (net GND)
  )
  (wiring
    (wire (rectangle In1.Cu 1000 1000 9000 9000) (net GND))
  )
)
"#;

    /// T47 composed shape (Task-1 trap, commit 502b88f8): a bare
    /// `(class ...)` inside `(network ...)` with `(wiring ...)` AFTER it,
    /// all closes balanced so the pcb scope completes. The wire is fully
    /// valid (F.Cu exists, net PERFECT exists, endpoints on the U1/U2 pin
    /// centers) — so a zero-trace result cannot be a wiring-scope veto.
    /// Jar probe /tmp/epic-t9fix-t47.jsh -> /tmp/epic-t9fix-t47.out.
    const T47_DSN: &str = r#"(pcb t47fix.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu
      (type signal)
    )
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (rule
      (width 250)
      (clearance 200)
    )
  )
  (placement
    (component PAD
      (place U1 2000 5000 front 0)
      (place U2 8000 5000 front 0)
    )
  )
  (library
    (image PAD
      (pin CirclePad 1 0 0)
    )
    (padstack CirclePad
      (shape (circle F.Cu 800))
      (attach off)
    )
  )
  (network
    (net PERFECT (pins U1-1 U2-1))
    (class kicad_default PERFECT
    )
  )
  (wiring
    (wire (path F.Cu 125  2000 5000  8000 5000) (net PERFECT)(type route))
  )
)
"#;

    fn run(fixture: &str) -> (crate::reader::DsnReadResult, SesBoard) {
        let mut board = SesBoard::new();
        let result = read_board(fixture.as_bytes(), &mut board);
        (result, board)
    }

    /// The 3-token header hand-scan (jar /tmp/epic-t9-res.out): any
    /// non-`(pcb` opener fails; `(` + keyword without a name still
    /// succeeds; the name is consumed in the forced NAME state (even a
    /// full scope after it is inert — the pcb scope dispatch starts at
    /// token 4).
    #[test]
    fn header_hand_scan_matches_jar_cases() {
        // "garbage" -> ParseError (header).
        let (result, board) = run(HEADER_GARBAGE_DSN);
        match result {
            crate::reader::DsnReadResult::ParseError { location, detail } => {
                assert_eq!(location, "(pcb");
                assert_eq!(
                    detail,
                    "Not a Specctra DSN file: expected '(pcb <name>' header"
                );
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
        assert_eq!(board.items.len(), 0);
        // "(pcb" -> Success (EOF after the keyword is fine).
        assert!(matches!(
            run(HEADER_BARE_PCB_DSN).0,
            crate::reader::DsnReadResult::Success { .. }
        ));
        // "(pcb x" -> Success (bare name).
        assert!(matches!(
            run(HEADER_PCB_NAME_DSN).0,
            crate::reader::DsnReadResult::Success { .. }
        ));
        // "(pcb x (resolution um 10))" -> Success (the trailing scope is
        // pcb-level dispatch).
        assert!(matches!(
            run(HEADER_PCB_EXTRA_DSN).0,
            crate::reader::DsnReadResult::Success { .. }
        ));
    }

    /// Jar /tmp/epic-t9-fresh.out CASE om: a structure without a boundary
    /// never creates a board -> OutlineMissing (the jar logs
    /// "Structure.create_board: outline missing at 'GND'").
    #[test]
    fn structure_without_boundary_is_outline_missing() {
        let (result, board) = run(OM_DSN);
        match result {
            crate::reader::DsnReadResult::OutlineMissing { warnings } => {
                assert!(warnings.is_empty());
            }
            other => panic!("expected OutlineMissing, got {other:?}"),
        }
        assert_eq!(board.items.len(), 0);
    }

    /// Jar /tmp/epic-t9-fresh3.out CASE parserx: the parser scope pins
    /// host_cad/host_version/string_quote; resolution mm 7 pins unit +
    /// resolution; the snap angle defaults to FORTYFIVE_DEGREE.
    #[test]
    fn parserx_metadata_pins() {
        let (result, board) = run(PARSERX_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.metadata.host_cad.as_deref(), Some("KICAD"));
        assert_eq!(board.metadata.host_version.as_deref(), Some("7.99"));
        assert_eq!(board.metadata.string_quote, "'");
        assert_eq!(board.metadata.unit, Unit::Mm);
        assert_eq!(board.metadata.resolution, 7);
        assert_eq!(board.metadata.snap_angle, AngleRestriction::FortyfiveDegree);
    }

    /// The T37 matrix. Plane areas are SYSTEM_FIXED when the heuristic
    /// fires (SYSTEM > USER, so a following promotion cannot override);
    /// the gate (an `(autoroute_settings ...)` scope anywhere) suppresses
    /// it; a PCB-level KiCad one-line scope is skip-scoped and does NOT
    /// close the gate.
    #[test]
    fn t37_plane_heuristic_matrix() {
        // FIRE + trace: heuristic fires on the plane net; area is
        // SYSTEM_FIXED; contains_plane true; GEN 3 (jar CASE t37fire).
        let (result, board) = run(T37FIRE_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.items.len(), 3);
        assert_eq!(board.items[1].id(), 2);
        match &board.items[1] {
            ItemIr::ConductionArea { area, .. } => {
                assert_eq!(area.layer_no, 1);
                assert_eq!(area.fixed, crate::sink::FixedStateIr::SystemFixed);
                assert_eq!(area.nets, vec![1]);
            }
            other => panic!("expected ConductionArea, got {other:?}"),
        }
        match &board.items[2] {
            ItemIr::Trace { trace, .. } => {
                assert_eq!(trace.layer_no, 0);
                assert_eq!(trace.fixed, crate::sink::FixedStateIr::Unfixed);
            }
            other => panic!("expected Trace, got {other:?}"),
        }
        assert!(board.nets[0].contains_plane);

        // GATED: same fixture with the one-line KiCad scope at PCB level —
        // identical outcome (jar CASE t37gated).
        let (result, board) = run(T37GATED_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.items.len(), 3);
        match &board.items[1] {
            ItemIr::ConductionArea { area, .. } => {
                assert_eq!(area.fixed, crate::sink::FixedStateIr::SystemFixed);
            }
            other => panic!("expected ConductionArea, got {other:?}"),
        }
        assert!(board.nets[0].contains_plane);

        // NOFIRE (plane fixture, extra In1.Cu trace): Java's
        // normalizeAllTraces removes the plane-layer UNFIXED trace but the
        // id is already burned -> jar GEN_MAX 4 with 3 stored items. The
        // port skips normalization (D11) and KEEPS the trace — same GEN 4,
        // 4 stored items. DOCUMENTED DIVERGENCE (phantom id, kept item).
        let (result, board) = run(T37NOFIRE_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.items.len(), 4);
        let max_id = board.items.iter().map(|item| item.id()).max();
        assert_eq!(max_id, Some(4));
        match &board.items[3] {
            ItemIr::Trace { trace, .. } => {
                assert_eq!(trace.layer_no, 1);
                assert_eq!(
                    trace.corners,
                    vec![
                        epic_geometry::int_point::IntPoint::new(20000, 20000),
                        epic_geometry::int_point::IntPoint::new(30000, 20000),
                    ]
                );
            }
            other => panic!("expected Trace, got {other:?}"),
        }
        assert!(board.nets[0].contains_plane);

        // FIRE2 (rectangle wire, no plane scope): the heuristic fires on
        // the rectangle wire itself -> USER_FIXED area (jar CASE t37fire2,
        // GEN 2, no traces).
        let (result, board) = run(T37FIRE2_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.items.len(), 2);
        match &board.items[1] {
            ItemIr::ConductionArea { area, .. } => {
                assert_eq!(area.layer_no, 1);
                assert_eq!(area.fixed, crate::sink::FixedStateIr::UserFixed);
            }
            other => panic!("expected ConductionArea, got {other:?}"),
        }
        assert!(board.nets[0].contains_plane);

        // GATE2: same with the one-line KiCad scope at PCB level —
        // identical (jar CASE t37gate2).
        let (result, board) = run(T37GATE2_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.items.len(), 2);
        match &board.items[1] {
            ItemIr::ConductionArea { area, .. } => {
                assert_eq!(area.fixed, crate::sink::FixedStateIr::UserFixed);
            }
            other => panic!("expected ConductionArea, got {other:?}"),
        }
        assert!(board.nets[0].contains_plane);

        // NOFIRE2: the extra In1.Cu TRACE (a small area would not count)
        // means the In1.Cu area is not >50% of the board's layer-1 area
        // coverage for the heuristic — area stays UNFIXED,
        // contains_plane false (jar CASE t37nofire2, GEN 3).
        let (result, board) = run(T37NOFIRE2_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.items.len(), 3);
        match &board.items[1] {
            ItemIr::ConductionArea { area, .. } => {
                assert_eq!(area.fixed, crate::sink::FixedStateIr::Unfixed);
            }
            other => panic!("expected ConductionArea, got {other:?}"),
        }
        match &board.items[2] {
            ItemIr::Trace { trace, .. } => {
                assert_eq!(trace.layer_no, 1);
                assert_eq!(
                    trace.corners,
                    vec![
                        epic_geometry::int_point::IntPoint::new(91000, 10000),
                        epic_geometry::int_point::IntPoint::new(98000, 10000),
                    ]
                );
            }
            other => panic!("expected Trace, got {other:?}"),
        }
        assert!(!board.nets[0].contains_plane);

        // FIRE2GATE: the multi-line scope INSIDE structure CLOSES the gate
        // -> area stays UNFIXED, contains_plane false (jar CASE
        // t37fire2gate, GEN 2).
        let (result, board) = run(T37FIRE2GATE_DSN);
        assert!(matches!(
            result,
            crate::reader::DsnReadResult::Success { .. }
        ));
        assert_eq!(board.items.len(), 2);
        match &board.items[1] {
            ItemIr::ConductionArea { area, .. } => {
                assert_eq!(area.fixed, crate::sink::FixedStateIr::Unfixed);
            }
            other => panic!("expected ConductionArea, got {other:?}"),
        }
        assert!(!board.nets[0].contains_plane);
    }

    /// T47 at the dispatcher level, jar /tmp/epic-t9fix-t47.out (probe
    /// /tmp/epic-t9fix-t47.jsh): Success, ZERO traces, ZERO warnings
    /// despite the fully valid wire. The jar consumed all 748 bytes —
    /// tokens past `(wiring` at offset 659 — so the parse walked past the
    /// wiring region and still produced no trace and no drop warning:
    /// the wiring scope never dispatched, and the loss is the NETWORK
    /// scope's bare-class close-eating cascade (t8_deg1/t8_deg2 in
    /// `scope/network.rs`), not a wiring veto. Side observations, same
    /// capture: the outline + both pins survive, and the bare class
    /// itself is lost too (NETCLASS_COUNT 1, only `default` — unlike the
    /// t8_deg1 end-of-network tail, where it applied).
    #[test]
    fn t47_class_before_wiring_cascade_parity() {
        let (result, board) = run(T47_DSN);
        match result {
            crate::reader::DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "cascade drops silently");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        let traces = board
            .items
            .iter()
            .filter(|item| matches!(item, ItemIr::Trace { .. }))
            .count();
        assert_eq!(
            traces, 0,
            "valid wire lost to the cascade, not a wiring veto"
        );
        assert_eq!(board.items.len(), 3);
        assert_eq!(
            board
                .items
                .iter()
                .filter(|item| matches!(item, ItemIr::BoardOutline { .. }))
                .count(),
            1
        );
        assert_eq!(
            board
                .items
                .iter()
                .filter(|item| matches!(item, ItemIr::Pin { .. }))
                .count(),
            2
        );
    }

    /// The T50 probe shape: the on-disk fixture /tmp/t9rev-eof.dsn ends
    /// `(mystery_scope_that_never_closes` + a stray `)` line — that close
    /// balances the mystery scope, so the fixture here omits it to
    /// actually reach the skip failure. The jar's CASE eof values hold
    /// for BOTH variants (ScopeKeyword.java:74-76 discards skipScope's
    /// result either way).
    const T9REV_EOF_DSN: &str = r#"(pcb t9rev-eof.dsn
  (structure
    (layer F.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
  )
  (mystery_scope_that_never_closes
"#;

    /// Jar /tmp/t9rev-two.out CASE eof: an unknown pcb-level scope hitting
    /// EOF does NOT fail the read — the dispatcher discards skip_scope's
    /// failure (`ScopeKeyword.java:74-76`), the loop's next token is EOF
    /// and readScope returns true (`:55-57`). Success, GEN_MAX 1 (the
    /// outline alone), WARN_COUNT 0. Before the fix the skip failure
    /// failed the read (ParseError).
    #[test]
    fn t50_eof_in_unknown_pcb_scope_is_discarded() {
        let (result, board) = run(T9REV_EOF_DSN);
        match result {
            crate::reader::DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        assert_eq!(board.items.len(), 1, "the outline alone");
        assert_eq!(board.items[0].id(), 1, "GEN_MAX 1");
    }
}
