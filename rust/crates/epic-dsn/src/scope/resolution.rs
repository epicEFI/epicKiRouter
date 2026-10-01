//! Port of `io.specctra.parser.Resolution` (`Resolution.java`): the
//! `(resolution <unit> <n>)` scope reader.
//!
//! `Resolution.readScope` (`:28-72`) is a strict three-token read — a
//! String unit name (`Unit.fromString`, case-insensitive per
//! `Unit.java:27-39`), a strict Integer, and the closing bracket — with
//! EVERY mismatch failing the read (the pcb dispatch then fails the whole
//! board read). The assignments are ORDERED: the unit lands BEFORE the
//! integer is attempted, so a failed integer read still leaves the unit
//! assigned (jar `/tmp/epic-t9-res.out` CASE resint: `READ_OK false
//! unit=um res=100`).
//!
//! Resolved divergence (Task 10): Java assigns `scopeParameter.unit` even
//! when `fromString` returns null (CASE resbad leaves `unit=null`); the
//! Rust [`crate::state::Unit`] is non-nullable, so the failed-unit read
//! keeps the previous value. Observable only through `readMetadata`'s
//! BoardMetadata (readOk=false dominates the read_board result either
//! way) — the port's `BoardMetadataIr::unit` (`reader.rs`) documents the
//! divergence instead of modeling Java's null.

use crate::lexer::{Scanner, Token};
use crate::state::ParseState;

/// Java `Resolution.readScope` (`:28-72`). `false` (Java false) on any
/// mismatch; every diagnostic is FRLogger-only (D12).
pub fn read_scope(scanner: &mut Scanner, state: &mut ParseState) -> bool {
    // read the unit — a String token (Resolution.java:31-38)
    let unit = match scanner.next_token() {
        Token::Str(name) => crate::state::Unit::from_name(&name),
        // Java: warn "Resolution.read_scope: string expected at '<id>'"
        // (log-only).
        _ => return false,
    };
    let Some(unit) = unit else {
        // Java: warn "Resolution.read_scope: unit mil, inch or mm expected
        // at '<id>'" (log-only; jar resbad). Java additionally overwrites
        // p.unit with null — see the module docs (resolved divergence,
        // Task 10).
        return false;
    };
    state.unit = unit;
    // read the scale factor — a strict Integer (:49-57)
    let resolution = match scanner.next_token() {
        Token::Int(value) => value,
        // Java: warn "Resolution.read_scope: integer expected at '<id>'"
        // (log-only; jar resint).
        _ => return false,
    };
    state.resolution = resolution;
    // overread the closing bracket (:59-66)
    if scanner.next_token() != Token::Close {
        // Java: warn "Resolution.read_scope: closing bracket expected at
        // '<id>'" (log-only; jar resclose).
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives `read_scope` positioned after the `resolution` keyword, the
    /// dispatcher's contract. Returns (read_ok, unit, resolution).
    fn run_resolution(body: &str) -> (bool, crate::state::Unit, i32) {
        let input = format!("(resolution {body})");
        let mut scanner = Scanner::new(input.as_bytes());
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(
            scanner.next_token(),
            Token::Keyword(crate::keyword::Keyword::Resolution)
        );
        let mut state = ParseState::default();
        let ok = read_scope(&mut scanner, &mut state);
        (ok, state.unit, state.resolution)
    }

    /// Jar `/tmp/epic-t9-res.out` CASE resok: `(resolution um 10)` reads
    /// clean — unit flips MIL -> UM and the resolution 100 -> 10. This pin
    /// is end-to-end load-bearing: the t47 fixtures' coordinate scaling
    /// (um 10: DSN 2000 -> board 20000) rides on it.
    #[test]
    fn valid_resolution_reads() {
        let (ok, unit, resolution) = run_resolution("um 10");
        assert!(ok);
        assert_eq!(unit, crate::state::Unit::Um);
        assert_eq!(resolution, 10);
    }

    /// Jar CASE resbad: an unknown unit name fails the read and leaves the
    /// resolution default. (Java also nulls `p.unit`; resolved divergence
    /// — module docs, Task 10.)
    #[test]
    fn unknown_unit_fails_read() {
        let (ok, unit, resolution) = run_resolution("bogus 10");
        assert!(!ok);
        assert_eq!(unit, crate::state::Unit::Mil);
        assert_eq!(resolution, 100);
    }

    /// Jar CASE resint: the unit assignment lands BEFORE the integer is
    /// attempted, so a failed integer read still leaves unit=UM while the
    /// resolution keeps its default.
    #[test]
    fn failed_integer_keeps_unit_assignment() {
        let (ok, unit, resolution) = run_resolution("um x");
        assert!(!ok);
        assert_eq!(unit, crate::state::Unit::Um);
        assert_eq!(resolution, 100);
    }

    /// Jar CASE resclose: both values are assigned before the closing
    /// bracket is checked; the extra token fails the read but the state
    /// keeps unit=UM res=10.
    #[test]
    fn close_mismatch_keeps_assignments() {
        let (ok, unit, resolution) = run_resolution("um 10 extra");
        assert!(!ok);
        assert_eq!(unit, crate::state::Unit::Um);
        assert_eq!(resolution, 10);
    }

    /// A non-String first token (an Integer) fails the read (Java
    /// `:32-38`).
    #[test]
    fn non_string_unit_fails_read() {
        let (ok, unit, resolution) = run_resolution("10 10");
        assert!(!ok);
        assert_eq!(unit, crate::state::Unit::Mil);
        assert_eq!(resolution, 100);
    }
}
