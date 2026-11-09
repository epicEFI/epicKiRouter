//! The `(placement ...)` scope reader: the port of `io/specctra/parser/
//! Placement.java` (which overrides nothing and inherits the generic
//! `ScopeKeyword.readScope` loop), `Component.java` (the `(component ...)`
//! and `(place ...)` readers), and `PlaceControl.java`.
//!
//! Java call shape: dispatched from the pcb-level loop as ScopeKeyword
//! instances; the `(component ...)` wrapper (`Component.java:367-380`)
//! appends each successful group to `scopeParameter.placementList`
//! ([`ParseState::placement_list`]). The flattened per-location
//! [`PlacementIr`] rows land on the sink at parse time (Task 8's
//! `insertComponents` consumes them in the same file order).
//!
//! Jar sessions (repo root, JDK 25 jar): `/tmp/epic-t7-place*.dsn` through
//! the full DsnReader, output `/tmp/epic-t7-probe.out` (`t7-place`,
//! `t7-place-bad`) and `/tmp/epic-t7-pc.dsn` (`t7-pc`).
//!
//! Documented divergences (all corpus-dead):
//! - The generic loop would recurse into ANY ScopeKeyword nested in a
//!   placement scope (`(structure ...)`, `(network ...)`, ... — Java
//!   dispatches on `instanceof ScopeKeyword`). Task 7 only owns
//!   `component`/`place_control`; every other scope is skip-scoped here.
//!   No corpus fixture nests pcb-level scopes inside `(placement ...)`;
//!   Task 9's table owns them at pcb level.
//! - `Token::Error(_)` (i32 overflow) is parse-fatal in every reader
//!   below: the Java scanner throws on overflow and the exception escapes
//!   uncaught (Task 1 jar pin).
//! - `readLockType` has NO EOF check (`Component.java:352-364`): at EOF
//!   Java's `nextToken()` keeps returning null and the oracle HANGS. The
//!   Rust scan must terminate, so EOF/error keeps the current result and
//!   returns.

use std::collections::BTreeMap;

use crate::keyword::{Keyword, skip_scope};
use crate::lexer::{LexicalState, Scanner, Token};
use crate::scope::structure::{read_flip_style_rotate_first, read_string_scope};
use crate::sink::{BoardSink, PlacementIr};
use crate::state::{ComponentLocation, ComponentPlacement, ItemClearanceInfo, ParseState};

/// The inherited `ScopeKeyword.readScope` loop (`ScopeKeyword.java:44-80`)
/// with `Placement` overriding nothing. Two EOF quirks are load-bearing:
/// EOF returns **true** (the truncated-scope parse SUCCEEDS —
/// `ScopeKeyword.java:52-55`),
/// and the `prev == OPEN` arm dispatches on ScopeKeyword instances.
///
/// `pub` (like [`crate::scope::structure::read_scope`]): the pcb-level
/// dispatcher that calls it lands with Task 9, and the crate keeps its
/// scope-reader entry points public until then.
pub fn read_placement_scope(
    scanner: &mut Scanner,
    state: &mut ParseState,
    sink: &mut dyn BoardSink,
) -> bool {
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if next_token == Token::Eof {
            // Java `nextToken == null` -> "end of file" -> return TRUE
            return true;
        }
        if matches!(next_token, Token::Error(_)) {
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            if let Token::Keyword(keyword) = next_token {
                match keyword {
                    Keyword::Component => {
                        if !read_component_scope(scanner, state, sink) {
                            return false;
                        }
                    }
                    Keyword::PlaceControl => {
                        if !read_place_control_scope(scanner, sink) {
                            return false;
                        }
                    }
                    // Java recurses into any other ScopeKeyword here; see the
                    // module-doc divergence note.
                    _ => {
                        skip_scope(scanner);
                    }
                }
            } else {
                // Java: a non-ScopeKeyword after `(` is skip-scoped
                skip_scope(scanner);
            }
        }
        prev_token = Some(next_token);
    }
    true
}

/// Java `Component.readScope` (`Component.java:29-54`). The walk has NO
/// else arm: only `prev == OPEN && next == PLACE` fires; every other token
/// just flows (bug-compat — an unknown inner scope is never skipped, its
/// tokens pass through the walk). EOF exits the loop and the placement is
/// KEPT (`while (... && nextToken != null)`); a failed `(place ...)` is
/// fatal and leaves the group unparsed.
pub(crate) fn read_component_scope(
    scanner: &mut Scanner,
    state: &mut ParseState,
    sink: &mut dyn BoardSink,
) -> bool {
    let Token::Str(component_name) = scanner.next_token() else {
        // Java warns "component name expected" -> null -> wrapper false
        state.note_parse_failure(
            "(component ...) scope: component image name expected after the \
             component keyword",
        );
        return false;
    };
    let mut placement = ComponentPlacement {
        lib_name: component_name.to_string(),
        locations: Vec::new(),
    };
    // Java seeds `prevToken` with the NAME token itself (the first inner
    // OPEN is consumed with prev = name).
    let mut prev_token = Token::Str(component_name);
    let mut next_token = scanner.next_token();
    while next_token != Token::Close && next_token != Token::Eof {
        if matches!(next_token, Token::Error(_)) {
            return false;
        }
        if prev_token == Token::Open && next_token == Token::Keyword(Keyword::Place) {
            match read_place_scope(scanner) {
                Some(location) => {
                    // Flattened per-location sink row (PlacementIr carries
                    // one location; ids are the 1-based positions in file
                    // order). Java keeps the GROUP on placementList and
                    // defers board insertion to Task 8 — a zero-location
                    // group appends no sink rows, matching Java's empty
                    // `locations` list.
                    sink.append_placement(PlacementIr {
                        lib_name: placement.lib_name.clone(),
                        location: location.clone(),
                    });
                    placement.locations.push(location);
                }
                None => {
                    state.note_parse_failure(format!(
                        "(place ...) scope inside (component \"{}\"): malformed — expected \
                         (place <refdes> <x> <y> [<front|back> <rotation>] [<inner scopes>])",
                        placement.lib_name
                    ));
                    return false;
                }
            }
        }
        prev_token = next_token;
        next_token = scanner.next_token();
    }
    state.placement_list.push(placement);
    true
}

/// Java `Component.readPlaceScope` (`Component.java:187-309`). `None` =
/// Java null = parse-fatal at the caller.
fn read_place_scope(scanner: &mut Scanner) -> Option<ComponentLocation> {
    let name = scanner.next_string_with(true, b' ');
    let mut coor = [0.0f64; 2];
    for slot_value in &mut coor {
        match scanner.next_token() {
            Token::Double(value) => *slot_value = value,
            Token::Int(value) => *slot_value = f64::from(value),
            Token::Close => {
                // "component is not yet placed": a CLOSE on the FIRST or
                // SECOND coordinate stores a location with a NULL coor,
                // front, rotation 0, nothing fixed (`:204-216`; jar
                // t7-place: `(place U9)` and `(place U10 500)` both land
                // `loc=null placed=false front=true rot=0.0`).
                return Some(ComponentLocation {
                    name,
                    coor: None,
                    is_front: true,
                    rotation: 0.0,
                    position_fixed: false,
                    pin_infos: BTreeMap::new(),
                    keepout_infos: BTreeMap::new(),
                    via_keepout_infos: BTreeMap::new(),
                    place_keepout_infos: BTreeMap::new(),
                    part_number: None,
                });
            }
            Token::Error(_) => return None,
            _ => {
                // Java warns "Double was expected as the second and third
                // parameter of the component/place command"
                return None;
            }
        }
    }
    let mut is_front = true;
    let mut rotation = 0.0;
    // Side + rotation. The Specctra grammar marks BOTH optional after the
    // coordinates; Java consumes whatever token follows as the side
    // (warn-only) and then requires a rotation — so the grammar-legal
    // minimal `(place R X Y)` and `(place R X Y <inner-scope> ...)` forms
    // died at the rotation read ("number expected"). Divergence
    // (2026-11-09): a token that is neither `front`/`back` nor a number
    // is now handled by shape — Close ends the placement (front, rotation
    // 0), Open flows into the inner-scope loop below, a number IS the
    // rotation. An EXPLICIT side keyword still requires a rotation token
    // (the t7-place-bad `(place ... front .5)` jar pin stays fatal), and
    // a side-token Error stays parse death.
    let mut next_token = scanner.next_token();
    match next_token {
        Token::Keyword(Keyword::Back) => {
            is_front = false;
            rotation = read_place_rotation(scanner)?;
            next_token = scanner.next_token();
        }
        Token::Keyword(Keyword::Front) => {
            rotation = read_place_rotation(scanner)?;
            next_token = scanner.next_token();
        }
        Token::Error(_) => {
            // The side token is LEXED before the warn-only front fallback
            // can apply: the scanner's NumberFormatException escapes every
            // IOException-only catch — parse death (jar
            // /tmp/epic-t8-errorpin.jsh, output /tmp/epic-t8-errorpin.out
            // t8-errorpin3: `(place U1 2000 2000 99999999999)` dies with
            // an uncaught NumberFormatException).
            return None;
        }
        Token::Double(value) => {
            rotation = value;
            next_token = scanner.next_token();
        }
        Token::Int(value) => {
            rotation = f64::from(value);
            next_token = scanner.next_token();
        }
        // Close (minimal `(place R X Y)`) and Open (an inner scope such
        // as `(mirror)`) keep the front/rotation-0 defaults; the loop
        // below consumes them.
        Token::Close | Token::Open => {}
        _ => {
            // Java warns "Keyword.FRONT expected" and CONTINUES with
            // front (`readPlacementSide` warn-only flavor) — but still
            // reads a rotation right after, which stays fatal on garbage
            // (t7-place-bad parity).
            rotation = read_place_rotation(scanner)?;
            next_token = scanner.next_token();
        }
    }
    let mut position_fixed = false;
    let mut part_number: Option<String> = None;
    let mut pin_infos = BTreeMap::new();
    let mut keepout_infos = BTreeMap::new();
    let mut via_keepout_infos = BTreeMap::new();
    let mut place_keepout_infos = BTreeMap::new();
    while next_token == Token::Open {
        let inner = scanner.next_token();
        if matches!(inner, Token::Error(_)) {
            return None;
        }
        match inner {
            Token::Keyword(Keyword::LockType) => position_fixed = read_lock_type(scanner),
            Token::Keyword(Keyword::Pin) => {
                let info = read_item_clearance_info(scanner)?;
                pin_infos.insert(info.name.clone(), info);
            }
            Token::Keyword(Keyword::Keepout) => {
                let info = read_item_clearance_info(scanner)?;
                keepout_infos.insert(info.name.clone(), info);
            }
            Token::Keyword(Keyword::ViaKeepout) => {
                let info = read_item_clearance_info(scanner)?;
                via_keepout_infos.insert(info.name.clone(), info);
            }
            Token::Keyword(Keyword::PlaceKeepout) => {
                let info = read_item_clearance_info(scanner)?;
                place_keepout_infos.insert(info.name.clone(), info);
            }
            // Java checks `Keyword.PN` OR a String equaling "PN"
            // case-insensitively (`:265-267`). The Rust keyword table has no
            // `pn` entry (bare `pn`/`PN` scan as strings — Task 1 sweep), so
            // the string arm covers both forms; quoted `"pn"` also lands
            // here, as in Java.
            Token::Str(ref s) if s.eq_ignore_ascii_case("pn") => {
                part_number = Some(read_string_scope(scanner));
            }
            // Hand-authored DSNs mark bottom-side parts with `(mirror)`
            // (KiCad's exporter spells the same intent as the `back` side
            // keyword). No keyword-table entry exists (Task 1 sweep), so
            // like `pn` it arrives as a string; the scope body carries
            // nothing else, so it is skipped after flipping the side.
            Token::Str(ref s) if s.eq_ignore_ascii_case("mirror") => {
                is_front = false;
                skip_scope(scanner);
            }
            _ => {
                skip_scope(scanner);
            }
        }
        next_token = scanner.next_token();
    }
    if next_token != Token::Close {
        // Java warns ") expected" — includes the EOF path
        return None;
    }
    Some(ComponentLocation {
        name,
        coor: Some(coor),
        is_front,
        rotation,
        position_fixed,
        pin_infos,
        keepout_infos,
        via_keepout_infos,
        place_keepout_infos,
        part_number,
    })
}

/// The rotation read after an EXPLICIT side keyword (or a garbage side
/// token that still owes one). Java warns "number expected" on anything
/// but a number — the t7-place-bad `(place ... front .5)` pin (`.5`
/// scans as a STRING token, no leading digit) stays fatal here.
fn read_place_rotation(scanner: &mut Scanner) -> Option<f64> {
    match scanner.next_token() {
        Token::Double(value) => Some(value),
        Token::Int(value) => Some(f64::from(value)),
        _ => None,
    }
}

/// Java `Component.readItemClearanceInfo` (`Component.java:311-350`).
/// `yybegin(NAME)` forces the info NAME to a string token; the clearance
/// class arm accepts the `clearance_class` keyword OR a string equaling
/// `clearance_class`/`clearanceClass` case-insensitively (the lexer state
/// after the name decides which form arrives — both are accepted). A
/// missing class is fatal.
fn read_item_clearance_info(scanner: &mut Scanner) -> Option<ItemClearanceInfo> {
    scanner.set_lexical_state(LexicalState::Name);
    let Token::Str(name) = scanner.next_token() else {
        // Java warns "String expected"
        return None;
    };
    let mut clearance_class: Option<String> = None;
    let mut next_token = scanner.next_token();
    while next_token == Token::Open {
        let inner = scanner.next_token();
        if matches!(inner, Token::Error(_)) {
            return None;
        }
        match inner {
            Token::Keyword(Keyword::ClearanceClass) => {
                clearance_class = Some(read_string_scope(scanner));
            }
            Token::Str(ref s)
                if s.eq_ignore_ascii_case("clearance_class")
                    || s.eq_ignore_ascii_case("clearanceclass") =>
            {
                clearance_class = Some(read_string_scope(scanner));
            }
            _ => {
                skip_scope(scanner);
            }
        }
        next_token = scanner.next_token();
    }
    if next_token != Token::Close {
        // Java warns ") expected" — includes the EOF path
        return None;
    }
    let clearance_class = clearance_class?;
    Some(ItemClearanceInfo {
        name: name.to_string(),
        clearance_class,
    })
}

/// Java `Component.readLockType` (`Component.java:352-364`): loops to the
/// closing bracket, `position` sets the flag. NO EOF check — see the
/// module-doc divergence note for the deliberate Rust termination.
fn read_lock_type(scanner: &mut Scanner) -> bool {
    let mut result = false;
    loop {
        match scanner.next_token() {
            Token::Close => break,
            Token::Keyword(Keyword::Position) => result = true,
            Token::Eof | Token::Error(_) => break,
            _ => {}
        }
    }
    result
}

/// Java `PlaceControl.readScope` (`PlaceControl.java:45-73`). Unlike the
/// generic loop this is hand-rolled: EOF is FATAL (`false`), and there is
/// NO else arm after the `flip_style` check — an unknown inner scope is
/// NOT skipped, so its tokens flow and its CLOSE ends THIS scope read
/// (bug-compat: the outer scope's close then flows to the pcb loop).
/// The `flip_style` result is ASSIGNED, so a failed read RESETS the flag;
/// the sink is only touched when it ends true (Java:
/// `components.setFlipStyleRotateFirst(true)` — set-only-when-true).
/// `pub` — the Task 9 pcb dispatcher owns the call site (see
/// [`read_placement_scope`]).
pub fn read_place_control_scope(scanner: &mut Scanner, sink: &mut dyn BoardSink) -> bool {
    let mut flip_style_rotate_first = false;
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java warns "unexpected end of file"
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) && next_token == Token::Keyword(Keyword::FlipStyle) {
            flip_style_rotate_first = read_flip_style_rotate_first(scanner);
        }
        prev_token = Some(next_token);
    }
    if flip_style_rotate_first {
        sink.set_flip_style("rotate_first".to_string());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ses_board::SesBoard;

    /// Scanner in the state the pcb dispatch leaves it in: right after the
    /// `placement` keyword (YyInitial — the keyword's action does not
    /// switch states).
    fn scanner_after_placement_keyword(input: &[u8]) -> Scanner<'_> {
        Scanner::new(input)
    }

    /// Jar `/tmp/epic-t7-probe.out` t7-place, PAD group: the placement
    /// values as PARSED (DSN coordinates; the board dump shows them
    /// transformed by the resolution — Task 8's insertComponents multiplies
    /// through `dsnToBoard`).
    #[test]
    fn t7_place_group_parses_all_location_shapes() {
        let input = b"\
(component PAD \
(place Z9 2000 2000 front 90 (pin 1 (clearance_class default)) \
(pin 1 (clearance_class power)) (pn PN-REV7) (lock_type position)) \
(place A1 3000 2000 back -45.5) \
(place U9) \
(place U10 500) \
(place R77 0 0 front 1.5e2)))";
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(input);
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(state.placement_list.len(), 1);
        let group = &state.placement_list[0];
        assert_eq!(group.lib_name, "PAD");
        assert_eq!(group.locations.len(), 5);

        // Z9: coords, rotation 90, FIXED, pn; T45 replace-on-duplicate pin
        // info: pin "1" read default THEN power — TreeMap put REPLACES, so
        // the placement map holds exactly ONE entry with class "power"
        // (jar: Z9 CPIN name=1 cls=2 — the appended power_default class;
        // the probe's OTHER CPIN, name=2 cls=1, is Task-8 board state —
        // the default info for the image's second pin — not a placement
        // map entry). A non-replacing map would hold 2 entries here — the
        // discriminating form.
        let z9 = &group.locations[0];
        assert_eq!(z9.name, "Z9");
        assert_eq!(z9.coor, Some([2000.0, 2000.0]));
        assert!(z9.is_front);
        assert_eq!(z9.rotation, 90.0);
        assert!(z9.position_fixed);
        assert_eq!(z9.part_number.as_deref(), Some("PN-REV7"));
        assert_eq!(z9.pin_infos.len(), 1);
        assert_eq!(z9.pin_infos["1"].clearance_class, "power");
        assert!(!z9.pin_infos.contains_key("2"));

        // A1: back side, negative fractional rotation (the parser value is
        // -45.5; the jar board dump's 314.5 is Task 8 insertion
        // normalization, not a parser concern).
        let a1 = &group.locations[1];
        assert_eq!(a1.coor, Some([3000.0, 2000.0]));
        assert!(!a1.is_front);
        assert_eq!(a1.rotation, -45.5);
        assert!(!a1.position_fixed);

        // U9 / U10: unplaced — null coor, front, rotation 0 (jar: `(place
        // U9)` closes on the FIRST coord, `(place U10 500)` on the SECOND;
        // both land placed=false).
        for unplaced in [&group.locations[2], &group.locations[3]] {
            assert_eq!(unplaced.coor, None);
            assert!(unplaced.is_front);
            assert_eq!(unplaced.rotation, 0.0);
            assert!(!unplaced.position_fixed);
            assert_eq!(unplaced.part_number, None);
        }

        // R77: scientific-notation rotation token 1.5e2 -> 150.0 (jar:
        // rot=150.0).
        let r77 = &group.locations[4];
        assert_eq!(r77.rotation, 150.0);

        // The sink holds the 6 flattened rows (5 + O1 would be a second
        // group; here 5) in file order.
        assert_eq!(board.placements.len(), 5);
        assert_eq!(board.placements[0].location.name, "Z9");
        assert_eq!(board.placements[4].location.name, "R77");
    }

    /// Jar t7-place, OTHER group: `(pn WIDGET-A)` string form.
    #[test]
    fn t7_place_second_group_and_pn() {
        let input = b"(component OTHER (place O1 4000 2000 front 0 (pn WIDGET-A))))";
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(input);
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(state.placement_list.len(), 1);
        let o1 = &state.placement_list[0].locations[0];
        assert_eq!(o1.part_number.as_deref(), Some("WIDGET-A"));
        assert_eq!(board.placements.len(), 1);
    }

    /// Jar t7-place-bad: `.5` scans as a STRING token (no leading digit),
    /// so the rotation arm warns "number expected" and the read is fatal.
    /// The discriminator: `0.5` (leading digit) parses fine.
    #[test]
    fn rotation_dot_five_is_fatal_but_zero_dot_five_parses() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(component OTHER (place O2 4000 2000 front .5)))");
        assert!(!read_placement_scope(&mut scanner, &mut state, &mut board));
        // fatal leaves NO group and NO sink rows
        assert!(state.placement_list.is_empty());
        assert!(board.placements.is_empty());

        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(component OTHER (place O2 4000 2000 front 0.5)))");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(state.placement_list[0].locations[0].rotation, 0.5);
    }

    /// A failed `(pin ...)` clearance info (missing class) is fatal; a
    /// complete one stores the class. Jar t7-place exercises the complete
    /// form (Z9 CPIN classes); the fatal form mirrors
    /// `Component.java:341-345` (warn "clearance class name not found" ->
    /// null -> fatal).
    #[test]
    fn pin_info_without_clearance_class_is_fatal() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(component PAD (place U1 0 0 front 0 (pin 1))))");
        assert!(!read_placement_scope(&mut scanner, &mut state, &mut board));
        assert!(state.placement_list.is_empty());
    }

    /// The inherited generic loop returns TRUE at EOF — a truncated
    /// placement scope SUCCEEDS and keeps everything read so far
    /// (`ScopeKeyword.java:47-51`). But EOF inside an open `(place ...)` is
    /// fatal (its final CLOSE check fails).
    #[test]
    fn eof_after_complete_place_keeps_the_group() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(b"(component PAD (place U1 0 0 front 0)");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(state.placement_list.len(), 1);
        assert_eq!(state.placement_list[0].locations.len(), 1);
        assert_eq!(board.placements.len(), 1);

        // EOF mid-place: the place's closing-bracket check fails -> fatal.
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(b"(component PAD (place U1 0 0");
        assert!(!read_placement_scope(&mut scanner, &mut state, &mut board));
        assert!(state.placement_list.is_empty());
    }

    /// Jar t7-pc: `(place_control (flip_style rotate_first))` sets
    /// FLIP_STYLE_ROTATE_FIRST true on the board. The bare `(flip_style)`
    /// form leaves it unset (result false — `readFlipStyleRotateFirst`
    /// finds no `rotate_first` token).
    #[test]
    fn t7_pc_place_control_flip_style() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(place_control (flip_style rotate_first)))");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.metadata.flip_style.as_deref(), Some("rotate_first"));

        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(b"(place_control (flip_style)))");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.metadata.flip_style, None);
    }

    /// A zero-location component group still lands on the placement list
    /// (Java adds the empty `ComponentPlacement`), with no sink rows.
    #[test]
    fn empty_component_group_appends_to_state_not_sink() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(b"(component PAD)))");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(state.placement_list.len(), 1);
        assert!(state.placement_list[0].locations.is_empty());
        assert!(board.placements.is_empty());
    }

    /// `read_place_control_scope` has NO else arm: an unknown inner scope
    /// is NOT skipped — its tokens flow and its CLOSE ends the
    /// place_control read, so a later `(flip_style rotate_first)` lands in
    /// the placement loop (which skip-scopes it) and never applies. Jar
    /// `/tmp/epic-t8-partb.out` t8-pc1: `(place_control (whatever 1)
    /// (flip_style rotate_first))` -> `FLIP_STYLE_ROTATE_FIRST false`
    /// (control t8-pc3 without the unknown scope: true).
    #[test]
    fn place_control_unknown_inner_scope_closes_the_read() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(
            b"(place_control (whatever 1) (flip_style rotate_first)))",
        );
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.metadata.flip_style, None, "read ended at (whatever");

        // Discriminator: without the unknown scope the flag applies.
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(place_control (flip_style rotate_first)))");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.metadata.flip_style.as_deref(), Some("rotate_first"));
    }

    /// The flip_style result is ASSIGNED (not OR-ed): a second, bare
    /// `(flip_style)` RESETS the flag. Jar `/tmp/epic-t8-partb.out` t8-pc2:
    /// `(place_control (flip_style rotate_first) (flip_style))` ->
    /// `FLIP_STYLE_ROTATE_FIRST false`.
    #[test]
    fn place_control_flip_style_is_assign_not_or() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(
            b"(place_control (flip_style rotate_first) (flip_style)))",
        );
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert_eq!(board.metadata.flip_style, None, "the bare read reset it");
    }

    /// Jar `/tmp/epic-t8-errorpin.out` t8-errorpin3: the side token of a
    /// `(place ...)` is LEXED before the warn-only front fallback — an
    /// overflow integer throws NumberFormatException uncaught, parse
    /// death. A garbage token that lexes cleanly keeps the warn-only front
    /// fallback (with the rotation still to come: `(place U1 2000 2000
    /// garbage 0)`).
    #[test]
    fn place_side_error_token_is_fatal_but_garbage_is_front() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(component PA (place U1 2000 2000 99999999999)))");
        assert!(!read_placement_scope(&mut scanner, &mut state, &mut board));
        assert!(state.placement_list.is_empty());

        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(component PA (place U1 2000 2000 garbage 0)))");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        assert!(state.placement_list[0].locations[0].is_front);
        assert_eq!(state.placement_list[0].locations[0].rotation, 0.0);
    }

    /// Real-world DSN tolerance (2026-11-09): the Specctra grammar marks
    /// side and rotation OPTIONAL after the coordinates. Java consumed
    /// the place-closing paren as the side token and died at the
    /// rotation read; the divergence accepts the minimal forms. The
    /// dropbear adapter DSN (hand-authored) shipped exactly these.
    #[test]
    fn minimal_place_without_side_or_rotation_parses_front_up() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner = scanner_after_placement_keyword(
            b"(component PA (place T1 10000 20000) (place T2 30000 40000 90)))",
        );
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        let group = &state.placement_list[0];
        assert_eq!(group.locations.len(), 2);
        let t1 = &group.locations[0];
        assert_eq!(t1.name, "T1");
        assert_eq!(t1.coor, Some([10000.0, 20000.0]));
        assert!(t1.is_front, "minimal form defaults to front");
        assert_eq!(t1.rotation, 0.0, "minimal form defaults to rotation 0");
        let t2 = &group.locations[1];
        assert_eq!(t2.rotation, 90.0, "bare rotation without a side keyword");
        assert!(t2.is_front);
    }

    /// Real-world DSN tolerance (2026-11-09): hand-authored DSNs mark
    /// bottom-side parts with `(mirror)` (KiCad's exporter spells the
    /// same intent as the `back` side keyword).
    #[test]
    fn place_mirror_scope_flips_the_side_to_back() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        let mut scanner =
            scanner_after_placement_keyword(b"(component TP (place V1 0 0 (mirror))))");
        assert!(read_placement_scope(&mut scanner, &mut state, &mut board));
        let v1 = &state.placement_list[0].locations[0];
        assert!(!v1.is_front, "(mirror) means the bottom side");
        assert_eq!(v1.rotation, 0.0);
    }

    /// The diagnostics face (2026-11-09): a malformed `(place ...)` now
    /// records WHICH component and the expected shape, instead of the
    /// generic "DSN structure parsing failed".
    #[test]
    fn malformed_place_records_component_named_detail() {
        let mut state = ParseState::default();
        let mut board = SesBoard::new();
        // Explicit side keyword then a NON-number rotation (t7-place-bad
        // parity) — still fatal, now with a named reason.
        let mut scanner =
            scanner_after_placement_keyword(b"(component IMG (place P1 100 200 front .5)))");
        assert!(!read_placement_scope(&mut scanner, &mut state, &mut board));
        let detail = state.parse_failure.as_deref().unwrap_or_default();
        assert!(
            detail.contains("(component \"IMG\")") && detail.contains("(place"),
            "detail names the component and the scope: {detail}"
        );
    }
}
