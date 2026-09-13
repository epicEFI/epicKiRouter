//! The rule scope readers: the port of `io/specctra/parser/Rule.java`
//! (`readScope`, `readWidthRule`, `readClearanceRule` — hoisted here from
//! the structure module so one module mirrors one Java parser class).
//!
//! Java call shape: `Structure.readScope` (`:980-982`) and the
//! `(layer <name> (rule ...))` arm (`:613-660`) both call
//! `Rule.readScope`, which loops over the scope body and collects
//! `WidthRule`/`ClearanceRule` flyweights; unknown inner scopes are
//! skip-scoped. The results feed `Structure.updateBoardRules`
//! (`:611-670`).
//!
//! ## Error discipline (Java crash parity)
//!
//! Three Java failure flavors, three outcomes:
//!
//! - `Rule.readScope` returning `null` (EOF/error token inside the rule
//!   scope) hits `defaultRules.addAll(null)` (`Structure.java:982`) —
//!   NPE, parse dies: [`read_rule_scope`] returns `None`.
//! - `readWidthRule`/`readClearanceRule` returning `null` (missing
//!   closing brackets, missing `(type` keyword) merely DROPS the rule:
//!   [`RuleOutcome::Skipped`].
//! - A `nextDouble` `null` (an unparseable number reaches the reader as
//!   a String) NPEs on unboxing inside the readers — parse dies:
//!   [`RuleOutcome::Fatal`].

use crate::keyword::{Keyword, skip_scope};
use crate::lexer::{LexicalState, Scanner, Token};

/// Java `Rule` (`parser/Rule.java:313-331`): the two rule kinds the
/// structure reader consumes (`WidthRule(value)`,
/// `ClearanceRule(value, clearanceClassPairs)`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Rule {
    /// Java `WidthRule`.
    Width {
        /// Java `value` (DSN units).
        value: f64,
    },
    /// Java `ClearanceRule`.
    Clearance {
        /// Java `value` (DSN units).
        value: f64,
        /// Java `clearanceClassPairs` — the `(type ...)` string list, empty
        /// when the rule has no type scope.
        pairs: Vec<String>,
    },
}

/// The outcome of reading ONE rule inside [`read_rule_scope`].
enum RuleOutcome {
    /// A rule was parsed.
    Parsed(Rule),
    /// Java produced a `null` rule (`readWidthRule`/`readClearanceRule`
    /// `return null`): the rule is dropped, the scope loop continues.
    Skipped,
    /// Java THREW (the `nextDouble` null-unboxing NPE): the whole parse
    /// dies.
    Fatal,
}

/// Java `Rule.readScope` (`Rule.java:24-64`). Returns `None` exactly when
/// Java returns `null` (EOF/error token inside the scope) — the caller
/// `Structure.java:982` NPEs on that (`addAll(null)`), so the structure
/// reader turns it into a parse failure.
pub(crate) fn read_rule_scope(scanner: &mut Scanner) -> Option<Vec<Rule>> {
    let mut result = Vec::new();
    let mut prev_token: Option<Token> = None;
    loop {
        let current_token = scanner.next_token();
        if matches!(current_token, Token::Eof | Token::Error(_)) {
            // Java: null nextToken -> warn + `return null` (the
            // IOException family behaves the same); the caller
            // addAll(null) NPEs.
            return None;
        }
        if current_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            let outcome = match current_token {
                Token::Keyword(Keyword::Width) => read_width_rule(scanner),
                Token::Keyword(Keyword::Clearance) => read_clearance_rule(scanner),
                _ => {
                    skip_scope(scanner);
                    RuleOutcome::Skipped
                }
            };
            match outcome {
                RuleOutcome::Parsed(rule) => result.push(rule),
                RuleOutcome::Skipped => {}
                RuleOutcome::Fatal => return None,
            }
        }
        prev_token = Some(current_token);
    }
    Some(result)
}

/// Java `LayerRule` (`parser/LayerRule.java`): the layer-name list and the
/// rules of one `(layer_rule ...)` scope of a net class.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerRuleIr {
    /// Java `LayerRule.layerNames`.
    pub layer_names: Vec<String>,
    /// Java `LayerRule.ruleList`.
    pub rules: Vec<Rule>,
}

/// The outcome of [`read_layer_rule_scope`]. Java's `null` is NOT a parse
/// death here: `NetClass.readScope` STORES null layer rules
/// (`NetClass.java:96` — `layerRules` is a LinkedList that allows null),
/// and the null kills LATER, during `insertNetClass`/`insertClassPairInfo`
/// iteration — AFTER the rules parsed before it were already applied
/// (ordering is load-bearing).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayerRuleOutcome {
    /// A parsed layer rule.
    Parsed(LayerRuleIr),
    /// Java returned null (warn-only) — store it and defer the death.
    Null,
    /// Java THREW (an Error token = the uncaught scanner
    /// NumberFormatException, or `readScope` null hitting
    /// `ruleList.addAll(null)`) — the whole parse dies immediately.
    Fatal,
}

/// Java `Rule.readLayerRuleScope` (`Rule.java:67-107`). Loop 1 forces
/// `yybegin(LAYER_NAME)` before EVERY token (so `pcb`/`signal` stay
/// keywords but every other layer name scans as a string): OPEN breaks,
/// a non-string (EOF null included) warns + returns null. Loop 2 reads
/// plain tokens: CLOSE breaks, a non-RULE warns + returns null, and a
/// RULE dispatches to [`read_rule_scope`] whose null hits
/// `ruleList.addAll(null)` — an immediate NPE death.
pub(crate) fn read_layer_rule_scope(scanner: &mut Scanner) -> LayerRuleOutcome {
    let mut layer_names = Vec::new();
    loop {
        scanner.set_lexical_state(LexicalState::LayerName);
        match scanner.next_token() {
            Token::Open => break,
            Token::Str(name) => layer_names.push(name.to_string()),
            Token::Error(_) => return LayerRuleOutcome::Fatal,
            _ => {
                // Java: null or a non-string token -> warn + `return null`
                return LayerRuleOutcome::Null;
            }
        }
    }
    let mut rule_list = Vec::new();
    loop {
        match scanner.next_token() {
            Token::Close => break,
            Token::Keyword(Keyword::Rule) => match read_rule_scope(scanner) {
                Some(rules) => rule_list.extend(rules),
                // `ruleList.addAll(null)` — uncaught NPE, parse death.
                None => return LayerRuleOutcome::Fatal,
            },
            Token::Error(_) => return LayerRuleOutcome::Fatal,
            _ => {
                // Java: `nextToken != Keyword.RULE` -> warn + `return null`
                return LayerRuleOutcome::Null;
            }
        }
    }
    LayerRuleOutcome::Parsed(LayerRuleIr {
        layer_names,
        rules: rule_list,
    })
}

/// Java `Rule.readWidthRule` (`Rule.java:109-117`): `(width <double>)`.
/// A `nextDouble` null (unparsable number; Java unboxing NPE) is
/// [`RuleOutcome::Fatal`]; a missing closing bracket makes the Java method
/// return `null` ([`RuleOutcome::Skipped`]).
fn read_width_rule(scanner: &mut Scanner) -> RuleOutcome {
    let Some(value) = scanner.next_double() else {
        return RuleOutcome::Fatal;
    };
    if !scanner.next_closing_bracket() {
        return RuleOutcome::Skipped;
    }
    RuleOutcome::Parsed(Rule::Width { value })
}

/// Java `Rule.readClearanceRule` (`Rule.java:257-303`):
/// `(clearance <double> [(type <pair list>)])`. The pair list is
/// `nextStringList(CLASS_CLEARANCE_SEPARATOR = '-')`
/// (`DsnFile.java:20`), so `power-ground` splits at the dash while
/// `c__d` arrives as one element. Both `nextClosingBracket` failures and
/// the `( expected`/`type expected` exits make Java return `null`
/// ([`RuleOutcome::Skipped`]); only a `nextDouble` null is a Java throw
/// ([`RuleOutcome::Fatal`]).
fn read_clearance_rule(scanner: &mut Scanner) -> RuleOutcome {
    let Some(value) = scanner.next_double() else {
        return RuleOutcome::Fatal;
    };
    let mut pairs = Vec::new();
    let next_token = scanner.next_token();
    if next_token != Token::Close {
        // look for "(type" (`Rule.java:263-275`)
        if next_token != Token::Open {
            return RuleOutcome::Skipped;
        }
        if scanner.next_token() != Token::Keyword(Keyword::Type) {
            return RuleOutcome::Skipped;
        }
        pairs = scanner.next_string_list_with(b'-');
        // the closing ")" of "(type" (`Rule.java:280-286`)
        if !scanner.next_closing_bracket() {
            return RuleOutcome::Skipped;
        }
        // the closing ")" of "(clearance" (`Rule.java:289-295`)
        if !scanner.next_closing_bracket() {
            return RuleOutcome::Skipped;
        }
    }
    RuleOutcome::Parsed(Rule::Clearance { value, pairs })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 5 jar `/tmp/epic-t5-probe.out` FILE 1 (the full rules battery)
    /// pins the reader through `updateBoardRules`; this module pins the
    /// RECOGNITION level for the `clear` abbreviation: it dispatches to
    /// the same rule as the canonical `clearance` spelling (jar
    /// `/tmp/epic-t6-alias.out`: `KW clear -> KW:clearance(st0)`). The
    /// wrong implementation (abbreviation skipped as an unknown scope)
    /// yields an EMPTY rule list and diverges.
    #[test]
    fn clear_abbreviation_matches_canonical() {
        let mut canonical = Scanner::new(b"(rule (clearance 600))");
        assert_eq!(canonical.next_token(), Token::Open);
        assert_eq!(canonical.next_token(), Token::Keyword(Keyword::Rule));
        let canonical_rules = read_rule_scope(&mut canonical).expect("rules");
        assert_eq!(
            canonical_rules,
            vec![Rule::Clearance {
                value: 600.0,
                pairs: Vec::new()
            }]
        );

        let mut abbreviated = Scanner::new(b"(rule (clear 600))");
        assert_eq!(abbreviated.next_token(), Token::Open);
        assert_eq!(abbreviated.next_token(), Token::Keyword(Keyword::Rule));
        let abbreviated_rules = read_rule_scope(&mut abbreviated).expect("rules");
        assert_eq!(abbreviated_rules, canonical_rules);
    }

    /// `read_layer_rule_scope` (`Rule.java:67-107`) three outcomes: the
    /// happy path collects the layer names then the rules; a non-RULE
    /// token after the names is Java null (STORED, deferred death — the
    /// outcome the NetClass reader must keep); EOF inside the `(rule ...)`
    /// scope makes `readScope` null, which hits `ruleList.addAll(null)` —
    /// immediate death.
    #[test]
    fn layer_rule_outcomes() {
        let mut scanner = Scanner::new(b"F.Cu B.Cu (rule (width 250)))");
        assert_eq!(
            read_layer_rule_scope(&mut scanner),
            LayerRuleOutcome::Parsed(LayerRuleIr {
                layer_names: vec!["F.Cu".to_string(), "B.Cu".to_string()],
                rules: vec![Rule::Width { value: 250.0 }],
            })
        );

        // A non-RULE token after the names: loop 2's warn+null arm —
        // Java null (STORED by NetClass.readScope, deferred death), not
        // an immediate failure.
        let mut scanner = Scanner::new(b"F.Cu (width 10))");
        assert_eq!(read_layer_rule_scope(&mut scanner), LayerRuleOutcome::Null);

        // An EMPTY rule scope after the names is a legitimate Parsed with
        // no rules (loop 2 breaks on CLOSE immediately).
        let mut scanner = Scanner::new(b"F.Cu ())");
        assert_eq!(
            read_layer_rule_scope(&mut scanner),
            LayerRuleOutcome::Parsed(LayerRuleIr {
                layer_names: vec!["F.Cu".to_string()],
                rules: Vec::new(),
            })
        );

        // EOF inside the rule scope: addAll(null) NPE -> death.
        let mut scanner = Scanner::new(b"F.Cu (rule");
        assert_eq!(read_layer_rule_scope(&mut scanner), LayerRuleOutcome::Fatal);

        // Keyword-colliding layer names: loop 1 re-arms LAYER_NAME before
        // EVERY token, where every keyword except `pcb`/`signal` degrades
        // to a string — `net` parses; `pcb` stays a KEYWORD and takes the
        // warn+null arm (Java-verbatim lexical table).
        let mut scanner = Scanner::new(b"net (rule (width 10)))");
        assert_eq!(
            read_layer_rule_scope(&mut scanner),
            LayerRuleOutcome::Parsed(LayerRuleIr {
                layer_names: vec!["net".to_string()],
                rules: vec![Rule::Width { value: 10.0 }],
            })
        );
        let mut scanner = Scanner::new(b"pcb (rule (width 10)))");
        assert_eq!(read_layer_rule_scope(&mut scanner), LayerRuleOutcome::Null);
    }
}
