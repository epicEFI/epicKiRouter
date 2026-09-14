//! Port of `io.specctra.parser.Keyword` / `ScopeKeyword` and the scanner's
//! keyword recognition table.
//!
//! Java models keywords as flyweight objects compared by reference
//! (`nextToken == Keyword.X`); the enum gives the same identity semantics.
//!
//! The recognition table below is **jar-captured** (jshell reflection sweep
//! over every `Keyword` constant, 2026-09-12, session
//! `/tmp/epic-keyword-sweep.jsh` against
//! `build/libs/freerouting-current-executable.jar`): for each constant the
//! input `(<name> 42)` was scanned and the produced token + post-scan lexical
//! state recorded. Three Java constants are *not* recognized by the
//! scanner's DFA and scan as plain `String` tokens: `PN`, `jumper`,
//! `polygon_path` (no enum variant here — the SES writer emits those names
//! as literal text). The fourth, `generated_by_freerouting`, is also
//! unrecognizable under its canonical spelling but IS reached through the
//! flex alias `generated_by_freeroute` (jar `/tmp/epic-t6-alias.out`), so
//! it exists as [`Keyword::GeneratedByFreerouting`] — the only variant
//! whose `name()` does not round-trip through [`Keyword::from_bytes`].
//! Conversely the word `path` is recognized as the [`Keyword::PolygonPath`]
//! keyword — the classic Specctra wire-shape alias (jar: `(path F.Cu ...)`
//! scans `KW:polygon_path`, and `(wire (path ...))` traces insert through
//! `read_wire_scope`'s polygon-path branch).
//!
//! Recognition is CASE-INSENSITIVE and abbreviation-friendly (jar
//! `/tmp/epic-t6-alias.out`: `PCB`, `Circle`, `KEEPOUT`, `Structure`,
//! `ROTATE_FIRST`, `On` all fold; the Specctra abbreviations `circ`,
//! `clear`, `comp`, `prefered_direction[-_trace_costs]`,
//! `against_prefered_direction_trace_costs`, `wire_keepout` and
//! `generated_by_freeroute` produce their canonical keywords), in the
//! YYINITIAL and LAYER_NAME states alike (`/tmp/epic-t6-alias2.out`).
//! The NAME state has NO keyword table at all (even `keepout` scans as a
//! string there). See [`Keyword::scanned_lexical_state`] for the
//! spelling-vs-keyword post-state split.

use crate::lexer::{LexicalState, Scanner, Token};

/// Keywords of the Specctra DSN format recognized by the scanner
/// (`Keyword.java` flyweights that the JFlex DFA actually matches).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyword {
    Absolute,
    Active,
    AgainstPreferredDirectionTraceCosts,
    Attach,
    Autoroute,
    AutorouteSettings,
    Back,
    Boundary,
    Circle,
    Circuit,
    Class,
    ClassClass,
    Classes,
    Clearance,
    ClearanceClass,
    Component,
    Constant,
    Control,
    Fanout,
    Fix,
    FlipStyle,
    GeneratedByFreerouting,
    FortyfiveDegree,
    Fromto,
    Front,
    Horizontal,
    HostCad,
    HostVersion,
    Image,
    Keepout,
    Layer,
    LayerRule,
    Length,
    Library,
    LockType,
    LogicalPart,
    LogicalPartMapping,
    Net,
    Network,
    NetworkOut,
    NinetyDegree,
    None,
    Normal,
    Off,
    On,
    Order,
    Outline,
    Padstack,
    Parser,
    PartLibrary,
    Pcb,
    Pin,
    Pins,
    Place,
    PlaceControl,
    PlaceKeepout,
    Placement,
    Plane,
    PlaneViaCosts,
    Polygon,
    PolygonPath,
    PolylinePath,
    Position,
    Postroute,
    Power,
    PreferredDirection,
    PreferredDirectionTraceCosts,
    PullTight,
    Rectangle,
    Resolution,
    Rotate,
    RotateFirst,
    Routes,
    Rule,
    Rules,
    Session,
    Shape,
    ShoveFixed,
    Side,
    Signal,
    SnapAngle,
    Spare,
    StartPassNo,
    StartRipupCosts,
    StringQuote,
    Structure,
    Type,
    UseLayer,
    UseNet,
    UseVia,
    Vertical,
    Via,
    ViaAtSmd,
    ViaCosts,
    ViaKeepout,
    ViaRule,
    Vias,
    Width,
    Window,
    Wire,
    Wiring,
    WriteResolution,
}

impl Keyword {
    /// The scanner recognition table for one already-lowercase word
    /// (jar-captured; see module docs). Includes the flex ABBREVIATION
    /// spellings, which produce the same Keyword flyweights as the
    /// canonical names (jar `/tmp/epic-t6-alias.out`).
    ///
    /// Only [`LexicalState::YyInitial`] consults the full table; the
    /// [`LexicalState::LayerName`] state recognizes only `pcb` and `signal`
    /// (jar: `(path signal 42)` scans `KW:signal`, `(path via 42)` scans
    /// `STR:"via"`) — see [`Scanner::next_token`].
    fn match_table(word: &[u8]) -> Option<Keyword> {
        let keyword = match word {
            b"absolute" => Keyword::Absolute,
            b"active" => Keyword::Active,
            b"against_prefered_direction_trace_costs"
            | b"against_preferred_direction_trace_costs" => {
                Keyword::AgainstPreferredDirectionTraceCosts
            }
            b"attach" => Keyword::Attach,
            b"autoroute" => Keyword::Autoroute,
            b"autoroute_settings" => Keyword::AutorouteSettings,
            b"back" => Keyword::Back,
            b"boundary" => Keyword::Boundary,
            // the Specctra circle abbreviation (jar alias sweep: `KW circ
            // -> KW:circle(st4)`)
            b"circ" | b"circle" => Keyword::Circle,
            b"circuit" => Keyword::Circuit,
            b"class" => Keyword::Class,
            b"class_class" => Keyword::ClassClass,
            b"classes" => Keyword::Classes,
            // the Specctra clearance abbreviation (jar: `KW clear ->
            // KW:clearance(st0)`); `Rule.readScope` dispatches on the
            // keyword, so `(rule (clear 600))` reads a clearance rule
            b"clear" | b"clearance" => Keyword::Clearance,
            b"clearance_class" => Keyword::ClearanceClass,
            // the Specctra component abbreviation (jar: `KW comp ->
            // KW:component(st3)`)
            b"comp" | b"component" => Keyword::Component,
            b"constant" => Keyword::Constant,
            b"control" => Keyword::Control,
            b"fanout" => Keyword::Fanout,
            b"fix" => Keyword::Fix,
            b"flip_style" => Keyword::FlipStyle,
            b"fortyfive_degree" => Keyword::FortyfiveDegree,
            b"fromto" => Keyword::Fromto,
            b"front" => Keyword::Front,
            b"horizontal" => Keyword::Horizontal,
            b"host_cad" => Keyword::HostCad,
            b"host_version" => Keyword::HostVersion,
            b"image" => Keyword::Image,
            b"keepout" => Keyword::Keepout,
            b"layer" => Keyword::Layer,
            b"layer_rule" => Keyword::LayerRule,
            b"length" => Keyword::Length,
            b"library" => Keyword::Library,
            b"lock_type" => Keyword::LockType,
            b"logical_part" => Keyword::LogicalPart,
            b"logical_part_mapping" => Keyword::LogicalPartMapping,
            b"net" => Keyword::Net,
            b"network" => Keyword::Network,
            b"network_out" => Keyword::NetworkOut,
            b"ninety_degree" => Keyword::NinetyDegree,
            b"none" => Keyword::None,
            b"normal" => Keyword::Normal,
            b"off" => Keyword::Off,
            b"on" => Keyword::On,
            b"order" => Keyword::Order,
            b"outline" => Keyword::Outline,
            b"padstack" => Keyword::Padstack,
            b"parser" => Keyword::Parser,
            b"part_library" => Keyword::PartLibrary,
            b"pcb" => Keyword::Pcb,
            b"pin" => Keyword::Pin,
            b"pins" => Keyword::Pins,
            b"place" => Keyword::Place,
            b"place_control" => Keyword::PlaceControl,
            b"place_keepout" => Keyword::PlaceKeepout,
            b"placement" => Keyword::Placement,
            b"plane" => Keyword::Plane,
            b"plane_via_costs" => Keyword::PlaneViaCosts,
            // jar session /tmp/epic-t5-rect2.jsh: the scanner ALSO accepts
            // the Specctra abbreviations `poly` and `rect` for
            // polygon/rectangle (both spellings produce the same Keyword
            // object); `polyline` and `polygon_path` are NOT keywords
            b"poly" => Keyword::Polygon,
            b"polygon" => Keyword::Polygon,
            // the Specctra wire-shape alias; `polygon_path` itself is NOT a
            // scanner keyword (sweep NOT-KW!)
            b"path" => Keyword::PolygonPath,
            b"polyline_path" => Keyword::PolylinePath,
            b"position" => Keyword::Position,
            b"postroute" => Keyword::Postroute,
            b"power" => Keyword::Power,
            // the one-token `r`-less spellings (jar alias sweep: `KW
            // prefered_direction -> KW:preferred_direction(st0)` and the
            // two `_trace_costs` variants)
            b"prefered_direction" | b"preferred_direction" => Keyword::PreferredDirection,
            b"prefered_direction_trace_costs" | b"preferred_direction_trace_costs" => {
                Keyword::PreferredDirectionTraceCosts
            }
            b"pull_tight" => Keyword::PullTight,
            b"rect" => Keyword::Rectangle,
            b"rectangle" => Keyword::Rectangle,
            b"resolution" => Keyword::Resolution,
            b"rotate" => Keyword::Rotate,
            b"rotate_first" => Keyword::RotateFirst,
            b"routes" => Keyword::Routes,
            b"rule" => Keyword::Rule,
            b"rules" => Keyword::Rules,
            b"session" => Keyword::Session,
            b"shape" => Keyword::Shape,
            b"shove_fixed" => Keyword::ShoveFixed,
            b"side" => Keyword::Side,
            b"signal" => Keyword::Signal,
            b"snap_angle" => Keyword::SnapAngle,
            b"spare" => Keyword::Spare,
            b"start_pass_no" => Keyword::StartPassNo,
            b"start_ripup_costs" => Keyword::StartRipupCosts,
            b"string_quote" => Keyword::StringQuote,
            b"structure" => Keyword::Structure,
            b"type" => Keyword::Type,
            b"use_layer" => Keyword::UseLayer,
            b"use_net" => Keyword::UseNet,
            b"use_via" => Keyword::UseVia,
            b"vertical" => Keyword::Vertical,
            b"via" => Keyword::Via,
            b"via_at_smd" => Keyword::ViaAtSmd,
            b"via_costs" => Keyword::ViaCosts,
            b"via_keepout" => Keyword::ViaKeepout,
            b"via_rule" => Keyword::ViaRule,
            b"vias" => Keyword::Vias,
            b"width" => Keyword::Width,
            b"window" => Keyword::Window,
            b"wire" => Keyword::Wire,
            b"wiring" => Keyword::Wiring,
            b"write_resolution" => Keyword::WriteResolution,
            // the keepout alias: same KEYWORD as `keepout` but a DIFFERENT
            // post-scan state — see [`Keyword::scanned_lexical_state`]
            b"wire_keepout" => Keyword::Keepout,
            // the `generated_by_freerouting` alias — the canonical spelling
            // is NOT a scanner keyword (sweep NOT-KW!), only this
            // abbreviated form is (jar alias sweep)
            b"generated_by_freeroute" => Keyword::GeneratedByFreerouting,
            _ => return None,
        };
        Some(keyword)
    }

    /// The scanner recognition table (jar-captured; see module docs).
    ///
    /// Recognition is ASCII case-insensitive (jar `/tmp/epic-t6-alias.out`:
    /// `PCB`, `Circle`, `KEEPOUT`, `Structure`, `ROTATE_FIRST`, `On` all
    /// fold to their keywords) — the word is lowercased before the table
    /// lookup; non-ASCII or oversized words are never keywords.
    pub fn from_bytes(name: &[u8]) -> Option<Keyword> {
        if let Some(keyword) = Self::match_table(name) {
            return Some(keyword);
        }
        if name.len() > 64 || !name.is_ascii() {
            return None;
        }
        let mut folded = [0u8; 64];
        for (slot, byte) in folded.iter_mut().zip(name) {
            *slot = byte.to_ascii_lowercase();
        }
        Self::match_table(&folded[..name.len()])
    }

    /// The lexical state the scanner switches to after scanning the word
    /// `spelling` as `keyword`. In the JFlex grammar the post-scan state
    /// lives on the ACTION (one per spelling), not on the shared Keyword
    /// flyweight — so an alias can share the keyword but keep a different
    /// state. The one divergent alias: `wire_keepout` produces
    /// [`Keyword::Keepout`] but stays in [`LexicalState::YyInitial`]
    /// (jar `/tmp/epic-t6-alias.out`: `(wire_keepout 42)` scans
    /// `KW:keepout(st0)` while `(keepout 42)` scans `KW:keepout(st3)`).
    /// Semantic effect: after `(wire_keepout `, a keyword-lookalike area
    /// NAME (e.g. `circle`) is recognized as a keyword instead of a
    /// string, so the keepout reads as unnamed — parse-level observable.
    pub fn scanned_lexical_state(spelling: &[u8], keyword: Keyword) -> LexicalState {
        if spelling.eq_ignore_ascii_case(b"wire_keepout") {
            return LexicalState::YyInitial;
        }
        keyword.lexical_state_after()
    }

    /// The scanner-visible name of this keyword (Java `getName()`); used for
    /// diagnostics only — recognition goes through [`Keyword::from_bytes`].
    /// [`Keyword::PolygonPath`] is named `path` (its scanner spelling); the
    /// Java constant's `getName()` is `polygon_path`, which the scanner does
    /// not recognize.
    pub fn name(self) -> &'static str {
        match self {
            Keyword::Absolute => "absolute",
            Keyword::Active => "active",
            Keyword::AgainstPreferredDirectionTraceCosts => {
                "against_preferred_direction_trace_costs"
            }
            Keyword::Attach => "attach",
            Keyword::Autoroute => "autoroute",
            Keyword::AutorouteSettings => "autoroute_settings",
            Keyword::Back => "back",
            Keyword::Boundary => "boundary",
            Keyword::Circle => "circle",
            Keyword::Circuit => "circuit",
            Keyword::Class => "class",
            Keyword::ClassClass => "class_class",
            Keyword::Classes => "classes",
            Keyword::Clearance => "clearance",
            Keyword::ClearanceClass => "clearance_class",
            Keyword::Component => "component",
            Keyword::Constant => "constant",
            Keyword::Control => "control",
            Keyword::Fanout => "fanout",
            Keyword::Fix => "fix",
            Keyword::FlipStyle => "flip_style",
            // the Java constant's name; the scanner spelling is the
            // `generated_by_freeroute` alias (see [`Keyword::from_bytes`])
            Keyword::GeneratedByFreerouting => "generated_by_freerouting",
            Keyword::FortyfiveDegree => "fortyfive_degree",
            Keyword::Fromto => "fromto",
            Keyword::Front => "front",
            Keyword::Horizontal => "horizontal",
            Keyword::HostCad => "host_cad",
            Keyword::HostVersion => "host_version",
            Keyword::Image => "image",
            Keyword::Keepout => "keepout",
            Keyword::Layer => "layer",
            Keyword::LayerRule => "layer_rule",
            Keyword::Length => "length",
            Keyword::Library => "library",
            Keyword::LockType => "lock_type",
            Keyword::LogicalPart => "logical_part",
            Keyword::LogicalPartMapping => "logical_part_mapping",
            Keyword::Net => "net",
            Keyword::Network => "network",
            Keyword::NetworkOut => "network_out",
            Keyword::NinetyDegree => "ninety_degree",
            Keyword::None => "none",
            Keyword::Normal => "normal",
            Keyword::Off => "off",
            Keyword::On => "on",
            Keyword::Order => "order",
            Keyword::Outline => "outline",
            Keyword::Padstack => "padstack",
            Keyword::Parser => "parser",
            Keyword::PartLibrary => "part_library",
            Keyword::Pcb => "pcb",
            Keyword::Pin => "pin",
            Keyword::Pins => "pins",
            Keyword::Place => "place",
            Keyword::PlaceControl => "place_control",
            Keyword::PlaceKeepout => "place_keepout",
            Keyword::Placement => "placement",
            Keyword::Plane => "plane",
            Keyword::PlaneViaCosts => "plane_via_costs",
            Keyword::Polygon => "polygon",
            Keyword::PolygonPath => "path",
            Keyword::PolylinePath => "polyline_path",
            Keyword::Position => "position",
            Keyword::Postroute => "postroute",
            Keyword::Power => "power",
            Keyword::PreferredDirection => "preferred_direction",
            Keyword::PreferredDirectionTraceCosts => "preferred_direction_trace_costs",
            Keyword::PullTight => "pull_tight",
            Keyword::Rectangle => "rectangle",
            Keyword::Resolution => "resolution",
            Keyword::Rotate => "rotate",
            Keyword::RotateFirst => "rotate_first",
            Keyword::Routes => "routes",
            Keyword::Rule => "rule",
            Keyword::Rules => "rules",
            Keyword::Session => "session",
            Keyword::Shape => "shape",
            Keyword::ShoveFixed => "shove_fixed",
            Keyword::Side => "side",
            Keyword::Signal => "signal",
            Keyword::SnapAngle => "snap_angle",
            Keyword::Spare => "spare",
            Keyword::StartPassNo => "start_pass_no",
            Keyword::StartRipupCosts => "start_ripup_costs",
            Keyword::StringQuote => "string_quote",
            Keyword::Structure => "structure",
            Keyword::Type => "type",
            Keyword::UseLayer => "use_layer",
            Keyword::UseNet => "use_net",
            Keyword::UseVia => "use_via",
            Keyword::Vertical => "vertical",
            Keyword::Via => "via",
            Keyword::ViaAtSmd => "via_at_smd",
            Keyword::ViaCosts => "via_costs",
            Keyword::ViaKeepout => "via_keepout",
            Keyword::ViaRule => "via_rule",
            Keyword::Vias => "vias",
            Keyword::Width => "width",
            Keyword::Window => "window",
            Keyword::Wire => "wire",
            Keyword::Wiring => "wiring",
            Keyword::WriteResolution => "write_resolution",
        }
    }

    /// True for the keywords whose Java classes extend `ScopeKeyword` — the
    /// scopes the pcb-level dispatch loop reads (`ScopeKeyword.readScope`,
    /// `ScopeKeyword.java:46-81`); every other token after `(` is
    /// skip-scoped (T24: that is why `(unit ...)` is dead).
    pub fn is_scope(self) -> bool {
        matches!(
            self,
            Keyword::Component
                | Keyword::Library
                | Keyword::Network
                | Keyword::PartLibrary
                | Keyword::Parser
                | Keyword::Pcb
                | Keyword::PlaceControl
                | Keyword::Placement
                | Keyword::Plane
                | Keyword::Resolution
                | Keyword::Structure
                | Keyword::Wiring
        )
    }

    /// Lexical state the scanner switches to after this keyword is scanned
    /// (the JFlex `yybegin` of the keyword's action). Jar-captured per
    /// keyword in the recognition sweep; keywords absent from the three
    /// non-initial groups leave the state at [`LexicalState::YyInitial`].
    pub fn lexical_state_after(self) -> LexicalState {
        match self {
            // st3 (NAME) group
            Keyword::Class
            | Keyword::ClearanceClass
            | Keyword::Component
            | Keyword::HostCad
            | Keyword::HostVersion
            | Keyword::Image
            | Keyword::Keepout
            | Keyword::Layer
            | Keyword::LayerRule
            | Keyword::LogicalPart
            | Keyword::LogicalPartMapping
            | Keyword::Net
            | Keyword::Padstack
            | Keyword::Place
            | Keyword::PlaceKeepout
            | Keyword::Plane
            | Keyword::UseLayer
            | Keyword::UseNet
            | Keyword::UseVia
            | Keyword::Via
            | Keyword::ViaKeepout
            | Keyword::Wire => LexicalState::Name,
            // st4 (LAYER_NAME) group: the shape keywords whose next word is
            // a layer name
            Keyword::Circle
            | Keyword::Polygon
            | Keyword::PolygonPath
            | Keyword::PolylinePath
            | Keyword::Rectangle => LexicalState::LayerName,
            // st7 (IGNORE_QUOTE)
            Keyword::StringQuote => LexicalState::IgnoreQuote,
            _ => LexicalState::YyInitial,
        }
    }
}

/// Port of `ScopeKeyword.skipScope` (`ScopeKeyword.java:21-43`): consumes
/// tokens until the scope's matching close bracket, returns `false` if no
/// legal scope was found (end of file or a scan error).
///
/// The scanner is forced into the [`LexicalState::Name`] state before *every*
/// token (`scanner.yybegin(NAME)`), so keyword-lookalikes and numbers inside
/// a skipped scope scan as plain strings — notably a huge integer like
/// `2147483648` does **not** abort the skip (it would throw
/// `NumberFormatException` in the initial state; jar behaviour, see the
/// `skip_scope_survives_overflowing_integer` test).
pub fn skip_scope(scanner: &mut Scanner) -> bool {
    let mut open_bracket_count = 1usize;
    while open_bracket_count > 0 {
        // the NAME state is forced before EVERY token, so keyword-lookalikes
        // and numbers inside a skipped scope scan as plain strings
        // (ScopeKeyword.java:24)
        scanner.set_lexical_state(LexicalState::Name);
        match scanner.next_token() {
            Token::Eof => return false, // end of file (ScopeKeyword.java:32-34)
            Token::Open => open_bracket_count += 1,
            Token::Close => open_bracket_count -= 1,
            Token::Error(_) => return false, // Java: catch-all -> false (:28-31)
            _ => {}
        }
    }
    scanner.set_lexical_state(LexicalState::YyInitial);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_token(s: &str) -> Token {
        Token::Str(Box::from(s))
    }

    /// Recognition sweep spot checks (jar session
    /// `/tmp/epic-keyword-sweep.jsh`, `KW <name> <state> ...` lines).
    #[test]
    fn recognition_table_spot_checks() {
        // "KW pcb 0 scope kw tok=ScopeKeyword"
        assert_eq!(Keyword::from_bytes(b"pcb"), Some(Keyword::Pcb));
        assert!(Keyword::Pcb.is_scope());
        // "KW component 3 scope kw tok=Component"
        assert_eq!(Keyword::from_bytes(b"component"), Some(Keyword::Component));
        assert!(Keyword::Component.is_scope());
        // "KW net 3 plain kw tok=Keyword"
        assert_eq!(Keyword::from_bytes(b"net"), Some(Keyword::Net));
        assert!(!Keyword::Net.is_scope());
        // "KW resolution 0 scope kw tok=Resolution"
        assert_eq!(
            Keyword::from_bytes(b"resolution"),
            Some(Keyword::Resolution)
        );
        // "KW structure 0 scope kw tok=Structure"
        assert_eq!(Keyword::from_bytes(b"structure"), Some(Keyword::Structure));
        assert_eq!(Keyword::from_bytes(b"wiring"), Some(Keyword::Wiring));
        assert_eq!(Keyword::from_bytes(b"placement"), Some(Keyword::Placement));
        assert_eq!(Keyword::from_bytes(b"library"), Some(Keyword::Library));
        assert_eq!(Keyword::from_bytes(b"network"), Some(Keyword::Network));
        assert_eq!(Keyword::from_bytes(b"parser"), Some(Keyword::Parser));
        assert_eq!(
            Keyword::from_bytes(b"part_library"),
            Some(Keyword::PartLibrary)
        );
        assert_eq!(
            Keyword::from_bytes(b"place_control"),
            Some(Keyword::PlaceControl)
        );
        assert_eq!(Keyword::from_bytes(b"plane"), Some(Keyword::Plane));
        // "KW type 0 plain kw tok=Keyword"
        assert_eq!(Keyword::from_bytes(b"type"), Some(Keyword::Type));
        assert_eq!(Keyword::from_bytes(b"signal"), Some(Keyword::Signal));
        assert_eq!(Keyword::from_bytes(b"fix"), Some(Keyword::Fix));
        assert_eq!(
            Keyword::from_bytes(b"shove_fixed"),
            Some(Keyword::ShoveFixed)
        );
        assert_eq!(Keyword::from_bytes(b"normal"), Some(Keyword::Normal));
        // unknown words are no keywords
        assert_eq!(Keyword::from_bytes(b"F.Cu"), None);
        assert_eq!(Keyword::from_bytes(b""), None);
    }

    /// Jar sweep: four Java `Keyword` constants are NOT scanner keywords
    /// under their canonical spellings and scan as `String` from the
    /// initial state ("NOT-KW!" sweep lines). `generated_by_freerouting`
    /// is nonetheless reachable through its `generated_by_freeroute`
    /// alias — see [`abbreviation_aliases`].
    #[test]
    fn unrecognizable_java_constants() {
        assert_eq!(Keyword::from_bytes(b"PN"), None);
        assert_eq!(Keyword::from_bytes(b"generated_by_freerouting"), None);
        assert_eq!(Keyword::from_bytes(b"jumper"), None);
        // "KW polygon_path 0 plain NOT-KW! tok=String" — the word
        // `polygon_path` is not a keyword; the keyword variant is reached
        // through the `path` alias instead.
        assert_eq!(Keyword::from_bytes(b"polygon_path"), None);
    }

    /// The flex ABBREVIATION spellings (jar session
    /// `/tmp/epic-t6-alias.out`, `KW <spelling> ...` lines): each produces
    /// the SAME Keyword flyweight as the canonical spelling. The
    /// wire_keepout line also pins the post-state divergence
    /// (`KW:keepout(st0)` vs canonical `(keepout 42)` -> `KW:keepout(st3)`).
    #[test]
    fn abbreviation_aliases() {
        // "KW circ -> OPEN(st0) KW:circle(st4) STR:"42"(st0)"
        assert_eq!(Keyword::from_bytes(b"circ"), Some(Keyword::Circle));
        // "KW clear -> OPEN(st0) KW:clearance(st0) INT:42(st0)"
        assert_eq!(Keyword::from_bytes(b"clear"), Some(Keyword::Clearance));
        // "KW comp -> OPEN(st0) KW:component(st3) STR:"42"(st0)"
        assert_eq!(Keyword::from_bytes(b"comp"), Some(Keyword::Component));
        // "KW prefered_direction -> ... KW:preferred_direction(st0)"
        assert_eq!(
            Keyword::from_bytes(b"prefered_direction"),
            Some(Keyword::PreferredDirection)
        );
        assert_eq!(
            Keyword::from_bytes(b"prefered_direction_trace_costs"),
            Some(Keyword::PreferredDirectionTraceCosts)
        );
        assert_eq!(
            Keyword::from_bytes(b"against_prefered_direction_trace_costs"),
            Some(Keyword::AgainstPreferredDirectionTraceCosts)
        );
        // "KW wire_keepout -> OPEN(st0) KW:keepout(st0) INT:42(st0)" —
        // the keyword is Keepout but the post-state stays YYINITIAL.
        assert_eq!(Keyword::from_bytes(b"wire_keepout"), Some(Keyword::Keepout));
        assert_eq!(
            Keyword::scanned_lexical_state(b"wire_keepout", Keyword::Keepout),
            LexicalState::YyInitial
        );
        // canonical keepout for contrast: NAME state (st3)
        assert_eq!(
            Keyword::scanned_lexical_state(b"keepout", Keyword::Keepout),
            LexicalState::Name
        );
        // "KW generated_by_freeroute -> ... KW:generated_by_freerouting(st0)"
        assert_eq!(
            Keyword::from_bytes(b"generated_by_freeroute"),
            Some(Keyword::GeneratedByFreerouting)
        );
        // ... while the canonical spelling scans as a string
        assert_eq!(Keyword::from_bytes(b"generated_by_freerouting"), None);
        // readBoard-level pins: jar `/tmp/epic-t6-board.out` —
        // `/tmp/epic-t6-alias.dsn` parses `(rule (clear 600))` into
        // clearance matrix M11 = 6000 on every layer (an unrecognized
        // abbreviation would leave 0), and `/tmp/epic-t6-t37-comp.dsn`
        // (alias `comp` inside `(placement ...)`) inserts the same board
        // as the canonical `(component ...)` fixture `t6-t37-fire.dsn`
        // (COMPONENT_COUNT 1, ITEMS 11 both).
    }

    /// Recognition is case-insensitive (jar `/tmp/epic-t6-alias.out`:
    /// `PCB`, `Circle`, `KEEPOUT`, `Structure`, `ROTATE_FIRST`, `On`; jar
    /// `/tmp/epic-t6-alias2.out`: `SIGNAL` folds inside the LAYER_NAME
    /// state too). Non-keywords stay non-keywords under folding.
    #[test]
    fn recognition_folds_ascii_case() {
        assert_eq!(Keyword::from_bytes(b"PCB"), Some(Keyword::Pcb));
        assert_eq!(Keyword::from_bytes(b"Circle"), Some(Keyword::Circle));
        assert_eq!(Keyword::from_bytes(b"KEEPOUT"), Some(Keyword::Keepout));
        assert_eq!(Keyword::from_bytes(b"Structure"), Some(Keyword::Structure));
        assert_eq!(
            Keyword::from_bytes(b"ROTATE_FIRST"),
            Some(Keyword::RotateFirst)
        );
        assert_eq!(Keyword::from_bytes(b"On"), Some(Keyword::On));
        assert_eq!(Keyword::from_bytes(b"SIGNAL"), Some(Keyword::Signal));
        // folded non-keywords are still no keywords (jar: `STR:"F.Cu"`)
        assert_eq!(Keyword::from_bytes(b"F.CU"), None);
        assert_eq!(Keyword::from_bytes(b"CIRCLE42"), None);
        // non-ASCII never folds into a keyword
        assert_eq!(Keyword::from_bytes(b"KEEPO\xc3\x9cT"), None);
    }

    /// Jar: `path` scans as the POLYGON_PATH keyword with LAYER_NAME
    /// post-state ("(wire (path F.Cu ...))" sweep: `KW:polygon_path st=4`),
    /// while `polyline_path`/`polygon`/`circle`/`rectangle` are their own
    /// keywords, also with LAYER_NAME post-state. The Specctra
    /// abbreviations `rect` and `poly` resolve to the SAME keywords as the
    /// long spellings (jar sessions `/tmp/epic-t5-rect.jsh` and
    /// `/tmp/epic-t5-rect2.jsh`: both `(rect ...)` and `(rectangle ...)`
    /// scan `KW:rectangle`, `poly` scans `KW:polygon`).
    #[test]
    fn path_aliases_and_shape_words() {
        assert_eq!(Keyword::from_bytes(b"path"), Some(Keyword::PolygonPath));
        assert_eq!(
            Keyword::from_bytes(b"polyline_path"),
            Some(Keyword::PolylinePath)
        );
        assert_eq!(Keyword::from_bytes(b"polygon"), Some(Keyword::Polygon));
        // the abbreviated spellings are scanner keywords too (Task 5 pins:
        // every /tmp/epic-t5-*.dsn boundary fixture uses `(rect pcb ...)`)
        assert_eq!(Keyword::from_bytes(b"poly"), Some(Keyword::Polygon));
        assert_eq!(Keyword::from_bytes(b"rect"), Some(Keyword::Rectangle));
        assert_eq!(Keyword::from_bytes(b"circle"), Some(Keyword::Circle));
        assert_eq!(Keyword::from_bytes(b"rectangle"), Some(Keyword::Rectangle));
        for kw in [
            Keyword::PolygonPath,
            Keyword::PolylinePath,
            Keyword::Polygon,
            Keyword::Circle,
            Keyword::Rectangle,
        ] {
            assert_eq!(kw.lexical_state_after(), LexicalState::LayerName);
        }
    }

    /// Post-state table spot checks (jar sweep: the `st` column after each
    /// `KW <name>` line).
    #[test]
    fn post_lexical_states() {
        use LexicalState::{IgnoreQuote, Name, YyInitial};
        // st3 (NAME) group
        for kw in [
            Keyword::Class,
            Keyword::ClearanceClass,
            Keyword::Component,
            Keyword::HostCad,
            Keyword::HostVersion,
            Keyword::Image,
            Keyword::Keepout,
            Keyword::Layer,
            Keyword::LayerRule,
            Keyword::LogicalPart,
            Keyword::LogicalPartMapping,
            Keyword::Net,
            Keyword::Padstack,
            Keyword::Place,
            Keyword::PlaceKeepout,
            Keyword::Plane,
            Keyword::UseLayer,
            Keyword::UseNet,
            Keyword::UseVia,
            Keyword::Via,
            Keyword::ViaKeepout,
            Keyword::Wire,
        ] {
            assert_eq!(kw.lexical_state_after(), Name, "kw {}", kw.name());
        }
        // st7 (IGNORE_QUOTE)
        assert_eq!(Keyword::StringQuote.lexical_state_after(), IgnoreQuote);
        // st0 groups (plain + scope keywords)
        for kw in [
            Keyword::Pcb,
            Keyword::Parser,
            Keyword::Placement,
            Keyword::Library,
            Keyword::Network,
            Keyword::Wiring,
            Keyword::Structure,
            Keyword::Resolution,
            Keyword::PartLibrary,
            Keyword::PlaceControl,
            Keyword::Type,
            Keyword::Signal,
        ] {
            assert_eq!(kw.lexical_state_after(), YyInitial, "kw {}", kw.name());
        }
        assert_eq!(Keyword::Layer.lexical_state_after(), Name);
    }

    /// Every keyword's `name()` must round-trip through `from_bytes` (the
    /// table and the names are two views of the same jar-captured data).
    ///
    /// [`Keyword::GeneratedByFreerouting`] is deliberately ABSENT from
    /// `ALL`: its canonical name is not a scanner spelling (its variant
    /// is reached only through the `generated_by_freeroute` alias — see
    /// [`abbreviation_aliases`]), so the round-trip property does not
    /// hold for it. Count stays at the jar-swept 101.
    #[test]
    fn names_round_trip() {
        const ALL: &[Keyword] = &[
            Keyword::Absolute,
            Keyword::Active,
            Keyword::AgainstPreferredDirectionTraceCosts,
            Keyword::Attach,
            Keyword::Autoroute,
            Keyword::AutorouteSettings,
            Keyword::Back,
            Keyword::Boundary,
            Keyword::Circle,
            Keyword::Circuit,
            Keyword::Class,
            Keyword::ClassClass,
            Keyword::Classes,
            Keyword::Clearance,
            Keyword::ClearanceClass,
            Keyword::Component,
            Keyword::Constant,
            Keyword::Control,
            Keyword::Fanout,
            Keyword::Fix,
            Keyword::FlipStyle,
            Keyword::FortyfiveDegree,
            Keyword::Fromto,
            Keyword::Front,
            Keyword::Horizontal,
            Keyword::HostCad,
            Keyword::HostVersion,
            Keyword::Image,
            Keyword::Keepout,
            Keyword::Layer,
            Keyword::LayerRule,
            Keyword::Length,
            Keyword::Library,
            Keyword::LockType,
            Keyword::LogicalPart,
            Keyword::LogicalPartMapping,
            Keyword::Net,
            Keyword::Network,
            Keyword::NetworkOut,
            Keyword::NinetyDegree,
            Keyword::None,
            Keyword::Normal,
            Keyword::Off,
            Keyword::On,
            Keyword::Order,
            Keyword::Outline,
            Keyword::Padstack,
            Keyword::Parser,
            Keyword::PartLibrary,
            Keyword::Pcb,
            Keyword::Pin,
            Keyword::Pins,
            Keyword::Place,
            Keyword::PlaceControl,
            Keyword::PlaceKeepout,
            Keyword::Placement,
            Keyword::Plane,
            Keyword::PlaneViaCosts,
            Keyword::Polygon,
            Keyword::PolygonPath,
            Keyword::PolylinePath,
            Keyword::Position,
            Keyword::Postroute,
            Keyword::Power,
            Keyword::PreferredDirection,
            Keyword::PreferredDirectionTraceCosts,
            Keyword::PullTight,
            Keyword::Rectangle,
            Keyword::Resolution,
            Keyword::Rotate,
            Keyword::RotateFirst,
            Keyword::Routes,
            Keyword::Rule,
            Keyword::Rules,
            Keyword::Session,
            Keyword::Shape,
            Keyword::ShoveFixed,
            Keyword::Side,
            Keyword::Signal,
            Keyword::SnapAngle,
            Keyword::Spare,
            Keyword::StartPassNo,
            Keyword::StartRipupCosts,
            Keyword::StringQuote,
            Keyword::Structure,
            Keyword::Type,
            Keyword::UseLayer,
            Keyword::UseNet,
            Keyword::UseVia,
            Keyword::Vertical,
            Keyword::Via,
            Keyword::ViaAtSmd,
            Keyword::ViaCosts,
            Keyword::ViaKeepout,
            Keyword::ViaRule,
            Keyword::Vias,
            Keyword::Width,
            Keyword::Window,
            Keyword::Wire,
            Keyword::Wiring,
            Keyword::WriteResolution,
        ];
        assert_eq!(ALL.len(), 101);
        for kw in ALL {
            assert_eq!(
                Keyword::from_bytes(kw.name().as_bytes()),
                Some(*kw),
                "kw {}",
                kw.name()
            );
        }
    }

    /// `skip_scope` consumes a balanced scope and resets to the initial
    /// state; it survives a huge integer because every token is scanned in
    /// the NAME state where numbers are strings (jar: `skipScope` forces
    /// `yybegin(NAME)` per token, `ScopeKeyword.java:24`; the integer would
    /// throw `NumberFormatException` from the initial state). Call order
    /// mirrors the reader: the open bracket and scope name are consumed as
    /// tokens BEFORE the skip dispatch (`skipScope` starts its bracket count
    /// at 1 for the already-seen open).
    #[test]
    fn skip_scope_survives_overflowing_integer() {
        let mut scanner = Scanner::new(b"(unit mm 2147483648 (nested (x))) (via)");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), str_token("unit"));
        assert!(skip_scope(&mut scanner));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        // the next tokens after the skipped scope open the `via` scope
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Via));
    }

    /// `skip_scope` returns false at end of file (`ScopeKeyword.java:32-34`)
    /// without consuming anything more. The state is NOT reset on the failure
    /// path — Java leaves the scanner in the NAME state forced before the
    /// last `nextToken` (`:24`); only the success path resets to YYINITIAL
    /// (`:41`).
    #[test]
    fn skip_scope_at_eof_returns_false() {
        let mut scanner = Scanner::new(b"(unit mm");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), str_token("unit"));
        assert!(!skip_scope(&mut scanner));
        assert_eq!(scanner.lexical_state(), LexicalState::Name);
        assert_eq!(scanner.next_token(), Token::Eof);
    }

    /// `skip_scope` idempotency (Task 12 invariant): each call consumes
    /// EXACTLY ONE balanced scope and resets to the initial state, so
    /// successive skips over adjacent scopes each advance by one scope —
    /// a skip must never over-consume into the sibling scope (that would
    /// silently drop real content, e.g. a second `(via ...)` scope after a
    /// skipped unknown scope in the pcb dispatch) and never under-consume
    /// (leaving bracket debt that corrupts the next dispatch).
    #[test]
    fn skip_scope_consumes_exactly_one_scope_per_call() {
        // The first scope nests two levels deep; the sibling scopes must
        // survive both skips byte-for-byte.
        let mut scanner = Scanner::new(b"(a (deep (deeper x))) (b) (tail)");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), str_token("a"));
        assert!(skip_scope(&mut scanner));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        // Position is exactly at the second scope's open bracket.
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), str_token("b"));
        assert!(skip_scope(&mut scanner));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        // The tail scope is untouched by the first skip.
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), str_token("tail"));
    }
}
