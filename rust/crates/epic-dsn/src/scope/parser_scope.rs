//! Port of `io.specctra.parser.Parser` (`Parser.java`): the `(parser ...)`
//! scope reader.
//!
//! Java structure: `Parser.readScope` (`:166-246`) is the standard
//! prev/next dispatch loop over six arms — `string_quote`, `host_cad`,
//! `host_version`, `constant`, `write_resolution`,
//! `generated_by_freerouting` — plus a generic `skipScope` fallback (which
//! is how `(space_in_quoted_tokens ...)` is consumed: there is NO arm for
//! it, jar `/tmp/epic-t9-parser.out` CASE parserx, 0 warnings).
//!
//! Jar-verified keyword truncation (Task 1 + `/tmp/epic-t9-gbf.out`,
//! 2026-09-13): the scanner keyword table maps the 22-char text
//! `generated_by_freeroute` to the flyweight the arm compares, while the
//! 24-char canonical spelling `generated_by_freerouting` — the one
//! `Parser.writeScope` EMITS (`Parser.java:142`) — is lexed as a plain
//! identifier. The arm is therefore reachable only through the alias
//! (jar `/tmp/epic-t9-parser.out`: CASE parserx keeps the default
//! `generatedByHost=true`, CASE parseralias flips it to `false`).
//!
//! Warnings: every diagnostic in `Parser.java` is FRLogger-only (D12) —
//! none of the six arms pushes a parity warning, so the Rust port has no
//! `state.warnings` traffic either.

use crate::keyword::{Keyword, skip_scope};
use crate::lexer::{LexicalState, Scanner, Token};
use crate::scope::structure::read_string_scope;
use crate::state::{ParseState, WriteResolution};

/// Java `Parser.readScope` (`:166-246`).
pub fn read_scope(scanner: &mut Scanner, state: &mut ParseState) -> bool {
    let mut prev_token: Option<Token> = None;
    loop {
        // Java: IOException -> warn (log-only) + false; null -> warn
        // (log-only) + false. Rust: both collapse to Token::Eof/Error.
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::StringQuote) => {
                    // Java `:183-190`: a null readQuoteChar fails the whole
                    // parser scope read (jar qe: `(string_quote ab c)` ->
                    // READ_OK false).
                    let Some(quote_char) = read_quote_char(scanner) else {
                        return false;
                    };
                    state.string_quote = quote_char;
                }
                Token::Keyword(Keyword::HostCad) => {
                    // Java `:191-193`: DsnFile.readStringScope never fails
                    // structurally (it drains to the bracket).
                    state.host_cad = Some(read_string_scope(scanner));
                }
                Token::Keyword(Keyword::HostVersion) => {
                    state.host_version = Some(read_string_scope(scanner));
                }
                Token::Keyword(Keyword::Constant) => {
                    // Java `:195-200`: a failed readConstant is SKIPPED —
                    // the parse continues (jar qf).
                    if let Some(constant) = read_constant(scanner) {
                        state.constants.push(constant);
                    }
                }
                Token::Keyword(Keyword::WriteResolution) => {
                    // Java `:201-203`: the assignment is UNCONDITIONAL — a
                    // failed read RESETS a previously stored resolution to
                    // null and the parse continues (jar qd).
                    state.write_resolution = read_write_solution(scanner);
                }
                Token::Keyword(Keyword::GeneratedByFreerouting) => {
                    // Java `:204-208`; reachable only through the 22-char
                    // `generated_by_freeroute` spelling (module docs).
                    state.dsn_file_generated_by_host = false;
                    // Java `:208`: the skipScope return value is DISCARDED.
                    let _ = skip_scope(scanner);
                }
                _ => {
                    // Java `:209-211`: unknown arm — generic skip. This is
                    // the path `(space_in_quoted_tokens ...)` takes.
                    let _ = skip_scope(scanner);
                }
            }
        }
        prev_token = Some(next_token);
    }
    true
}

/// Java `Parser.readQuoteChar` (`:147-165`): one String token (the quote
/// character OR a longer string — jar `/tmp/epic-t9-quote2.out` CASE qa
/// stores `'a` as the two-character quote string), then the closing
/// bracket. `None` on either mismatch.
fn read_quote_char(scanner: &mut Scanner) -> Option<String> {
    match scanner.next_token() {
        Token::Str(result) => {
            if scanner.next_token() != Token::Close {
                // Java: warn "Parser.read_quote_char: closing bracket
                // expected at '<id>'" (log-only; jar qe).
                return None;
            }
            Some(result.to_string())
        }
        // Java: warn "Parser.read_quote_char: string expected at '<id>'"
        // (log-only).
        _ => None,
    }
}

/// Java `Parser.readConstant` (`:53-91`): two NAME-forced String tokens and
/// the closing bracket. The NAME forcing is load-bearing — `(constant abc
/// 5)` stores `"5"` as a STRING (jar `/tmp/epic-t9-parser.out` CASE
/// parserx `constants=[[abc, 5], [foo, bar]]`; jar `/tmp/epic-t9-quote2.out`
/// CASE qc `[[5, 6]]`), and a short read is skipped without failing the
/// parse (jar `/tmp/epic-t9-quote3.out` CASE qf).
fn read_constant(scanner: &mut Scanner) -> Option<[String; 2]> {
    let mut result: [String; 2] = Default::default();
    for slot in &mut result {
        // Java `:59`/`:67`: yybegin(NAME) before each value token.
        scanner.set_lexical_state(LexicalState::Name);
        match scanner.next_token() {
            Token::Str(value) => *slot = value.to_string(),
            // Java: warn "Parser.read_constant: string expected at '<id>'"
            // (log-only; jar qf).
            _ => return None,
        }
    }
    // Java `:73`: the bracket is read WITHOUT re-forcing NAME — the
    // second NAME-state read already restored YYINITIAL.
    if scanner.next_token() != Token::Close {
        // Java: warn "Parser.read_constant: closing_bracket expected at
        // '<id>'" (log-only).
        return None;
    }
    Some(result)
}

/// Java `Parser.readWriteSolution` (`:18-50`): a String unit name, a strict
/// Integer, and the closing bracket. A strict-Integer failure is the jar
/// qd path: `None` propagates and the caller's unconditional assignment
/// resets the state field.
fn read_write_solution(scanner: &mut Scanner) -> Option<WriteResolution> {
    let resolution_string = match scanner.next_token() {
        Token::Str(value) => value.to_string(),
        // Java: warn "Parser.read_write_solution: string expected at
        // '<id>'" (log-only).
        _ => return None,
    };
    let resolution_value = match scanner.next_token() {
        Token::Int(value) => value,
        // Java: warn "Parser.read_write_solution: integer expected expected
        // at '<id>'" — the doubled word is in the jar (log-only; jar qd).
        _ => return None,
    };
    if scanner.next_token() != Token::Close {
        // Java: warn "Parser.read_write_solution: closing_bracket expected
        // at '<id>'" (log-only).
        return None;
    }
    Some(WriteResolution {
        char_name: resolution_string,
        positive_int: resolution_value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives `read_scope` positioned after the `parser` keyword (the
    /// dispatcher's contract), like the jar probe sessions positioned the
    /// scanner via `Keyword.PCB_SCOPE.readScope(p)` (the full pcb dispatch
    /// reaches `Parser.readScope` through the same `prevToken == OPEN &&
    /// nextToken instanceof ScopeKeyword` arm).
    fn run_parser(body: &str) -> (bool, ParseState) {
        let input = format!("(parser {body})");
        let mut scanner = Scanner::new(input.as_bytes());
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Parser));
        let mut state = ParseState::default();
        let ok = read_scope(&mut scanner, &mut state);
        (ok, state)
    }

    /// Jar `/tmp/epic-t9-parser.out` CASE parserx
    /// (`/tmp/epic-t9-parserx.dsn`): the full six-arm battery. Pins
    /// T44: stringQuote `'`, hostCad `KICAD`, hostVersion `7.99` (a
    /// NAME-state string, not a number), constants `[[abc, 5], [foo, bar]]`
    /// (the `5` is a STRING — readConstant's NAME forcing), writeResolution
    /// `um 10`, and `generatedByHost` STAYS true — the 24-char
    /// `(generated_by_freerouting)` spelling is a plain identifier to the
    /// scanner, so the arm never fires on it (module docs).
    #[test]
    fn parser_scope_full_battery_jar_probe() {
        let (ok, state) = run_parser(
            "(string_quote ')\
             (host_cad KICAD)\
             (host_version 7.99)\
             (constant abc 5)\
             (constant foo bar)\
             (write_resolution um 10)\
             (generated_by_freerouting)\
             (space_in_quoted_tokens on)",
        );
        assert!(ok);
        assert_eq!(state.string_quote, "'");
        assert_eq!(state.host_cad.as_deref(), Some("KICAD"));
        assert_eq!(state.host_version.as_deref(), Some("7.99"));
        assert_eq!(
            state.constants,
            vec![
                ["abc".to_string(), "5".to_string()],
                ["foo".to_string(), "bar".to_string()],
            ]
        );
        assert_eq!(
            state.write_resolution,
            Some(WriteResolution {
                char_name: "um".to_string(),
                positive_int: 10,
            })
        );
        assert!(state.dsn_file_generated_by_host);
    }

    /// Jar `/tmp/epic-t9-parser.out` CASE parseralias
    /// (`/tmp/epic-t9-parseralias.dsn`): the 22-char alias spelling
    /// `(generated_by_freeroute)` lexes as the
    /// [`Keyword::GeneratedByFreerouting`] flyweight and the arm FIRES —
    /// `dsn_file_generated_by_host` flips to false. This is the ONLY
    /// spelling that reaches the arm (anchor-blind partner of the
    /// full-battery test above).
    #[test]
    fn generated_by_alias_flips_flag() {
        let (ok, state) = run_parser("(generated_by_freeroute)");
        assert!(ok);
        assert!(!state.dsn_file_generated_by_host);
    }

    /// Jar `/tmp/epic-t9-parser.out` CASE resmm
    /// (`/tmp/epic-t9-resmm.dsn`): `(parser (string_quote "))` stores the
    /// default quote char explicitly; every other field keeps its
    /// `ReadScopeParameter` default (`:67-89`).
    #[test]
    fn minimal_parser_keeps_defaults() {
        let (ok, state) = run_parser("(string_quote \")");
        assert!(ok);
        assert_eq!(state.string_quote, "\"");
        assert_eq!(state.host_cad, None);
        assert_eq!(state.host_version, None);
        assert!(state.constants.is_empty());
        assert_eq!(state.write_resolution, None);
        assert!(state.dsn_file_generated_by_host);
    }

    /// Jar `/tmp/epic-t9-quote2.out` CASES qa/qb
    /// (`/tmp/epic-t9-qa.dsn`, `/tmp/epic-t9-qb.dsn`): the quote-char read
    /// takes ONE IGNORE_QUOTE-state identifier token, whatever its length —
    /// `'a` stores the TWO-character quote string `'a` and `ab` stores
    /// `ab`. An implementation that reads a single char (or strips quote
    /// characters) diverges from both pins.
    #[test]
    fn quote_char_keeps_full_token() {
        let (ok, state) = run_parser("(string_quote 'a)");
        assert!(ok);
        assert_eq!(state.string_quote, "'a");
        let (ok, state) = run_parser("(string_quote ab)");
        assert!(ok);
        assert_eq!(state.string_quote, "ab");
    }

    /// Jar `/tmp/epic-t9-quote2.out` CASE qc (`/tmp/epic-t9-qc.dsn`):
    /// `(constant 5 6)` — BOTH value tokens NAME-forced to strings, so the
    /// stored constant is `["5", "6"]`, not integers and not a failed read.
    /// A port that skips the NAME forcing reads `Token::Int(5)` first and
    /// (wrongly) drops the constant.
    #[test]
    fn constant_name_forcing_stringifies_numbers() {
        let (ok, state) = run_parser("(constant 5 6)");
        assert!(ok);
        assert_eq!(state.constants, vec![["5".to_string(), "6".to_string()]]);
    }

    /// Jar `/tmp/epic-t9-quote3.out` CASE qd (`/tmp/epic-t9-qd.dsn`):
    /// `(write_resolution um x)` — the second token is a String, not an
    /// Integer; readWriteSolution returns null AND the assignment still
    /// happens, so a previously stored resolution resets to None; the parse
    /// continues (`READ_OK true`).
    #[test]
    fn write_resolution_failure_resets_field() {
        let (ok, state) = run_parser("(write_resolution um 10)(write_resolution um x)");
        assert!(ok);
        // The failed read RESET the good value from the first scope.
        assert_eq!(state.write_resolution, None);
    }

    /// Jar `/tmp/epic-t9-quote3.out` CASE qe (`/tmp/epic-t9-qe.dsn`):
    /// `(string_quote ab c)` — the third token is not the closing bracket;
    /// readQuoteChar returns null and the WHOLE parser scope read fails
    /// (`READ_OK false`), unlike every other arm's tolerate-and-continue.
    #[test]
    fn quote_char_close_mismatch_fails_parse() {
        let (ok, _state) = run_parser("(string_quote ab c)");
        assert!(!ok);
    }

    /// Jar `/tmp/epic-t9-quote3.out` CASE qf (`/tmp/epic-t9-qf.dsn`):
    /// `(constant a)` — the second value token is missing (the closing
    /// bracket arrives instead); the constant is dropped but the parse
    /// continues (`READ_OK true`, empty constants).
    #[test]
    fn short_constant_is_skipped() {
        let (ok, state) = run_parser("(constant a)");
        assert!(ok);
        assert!(state.constants.is_empty());
    }
}
