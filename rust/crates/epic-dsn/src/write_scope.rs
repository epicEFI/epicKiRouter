//! Seed of the SES writer port (Task 13's `write_scope.rs` in the M1b
//! plan): the identifier-quoting rule. It lives in epic-dsn because BOTH
//! consumers need it — the harness parse digest (`dsn_digest`) and the
//! Task 13 SES writer — and the dependency runs harness -> epic-dsn only,
//! so a harness-local copy could never be shared with the writer.

/// The SES reserved-character set (`SesWriter.java:62`); a name carrying
/// any of these is written quoted.
pub const SES_RESERVED_CHARS: [char; 10] = ['(', ')', ' ', ';', '-', '_', '/', '~', '{', '}'];

/// Java `IdentifierType.write` (`datastructures/IdentifierType.java`,
/// T38), the SES identifier rule shared by the digest's `<padstack>`
/// names and the Task 13 writer:
///
/// 1. de-quote loop — while the name is longer than 2 UTF-16 units and
///    `"`-wrapped, `name = substring(1, length - 2)`. OFF-BY-ONE KEPT
///    (T38): the cut drops the closing quote AND one extra char, so
///    `"abc"` becomes `ab` and `"a"` becomes empty.
/// 2. remove every occurrence of the string quote char.
/// 3. quote if any SES reserved char is present (see
///    [`SES_RESERVED_CHARS`]).
/// 4. quote if any UTF-8 byte is `<= 0` as a SIGNED Java byte (i.e.
///    `0x00` or `>= 0x80` — any non-ASCII byte).
/// 5. otherwise quote if the name matches `^-?\d.*` (ASCII digit first,
///    optionally after one `-`).
///
/// The UTF-16 unit loop mirrors Java's `charAt`/`length` exactly; for a
/// name whose de-quote cut splits a surrogate pair Java keeps the lone
/// surrogate while this port substitutes U+FFFD (`from_utf16_lossy`) —
/// unreachable for the corpus name space, revisited if a fixture ever
/// produces it.
pub fn quote_java_identifier(name: &str, string_quote: &str) -> String {
    // 1. the T38 off-by-one de-quote loop, in UTF-16 units
    let mut units: Vec<u16> = name.encode_utf16().collect();
    while units.len() > 2
        && units[0] == u16::from(b'"')
        && units[units.len() - 1] == u16::from(b'"')
    {
        units = units[1..units.len() - 2].to_vec();
    }
    let mut name = String::from_utf16_lossy(&units);

    // 2. strip the string quote character
    if !string_quote.is_empty() && name.contains(string_quote) {
        name = name.replace(string_quote, "");
    }

    // 3.-5. the quoting decision
    let need_quotes = name.chars().any(|c| SES_RESERVED_CHARS.contains(&c))
        || name.as_bytes().iter().any(|&b| b == 0 || b & 0x80 != 0)
        || {
            let mut chars = name.chars();
            match chars.next() {
                Some(first) if first.is_ascii_digit() => true,
                Some('-') => chars.next().is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            }
        };
    if need_quotes {
        format!("{string_quote}{name}{string_quote}")
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `IdentifierType.write` (T38) pins — the SES reserved set of
    /// `SesWriter.java:62` quotes `_`, and the de-quote loop has the
    /// off-by-one (module docs): `"abc"` -> `ab`, `"ab"` -> `a`,
    /// `"a"` -> empty. Digit-start names quote WITHOUT carrying any
    /// reserved char; a lone `-` prefix quotes only before a digit via
    /// the regex arm — but `-` itself is reserved, so the discriminating
    /// digit cases are `1abc` (regex-only) vs `abc`.
    #[test]
    fn quote_java_identifier_t38() {
        assert_eq!(quote_java_identifier("abc", "\""), "abc");
        assert_eq!(quote_java_identifier("a b", "\""), "\"a b\"");
        assert_eq!(quote_java_identifier("a-b", "\""), "\"a-b\"");
        assert_eq!(quote_java_identifier("ViaPad_F", "\""), "\"ViaPad_F\"");
        assert_eq!(quote_java_identifier("1abc", "\""), "\"1abc\"");
        assert_eq!(quote_java_identifier("-1abc", "\""), "\"-1abc\"");
        // T38 off-by-one: one extra char is dropped per iteration
        assert_eq!(quote_java_identifier("\"abc\"", "\""), "ab");
        assert_eq!(quote_java_identifier("\"ab\"", "\""), "a");
        assert_eq!(quote_java_identifier("\"a\"", "\""), "");
        assert_eq!(quote_java_identifier("ab", "\""), "ab");
        // embedded quote chars are stripped before the decision
        assert_eq!(quote_java_identifier("a\"b", "\""), "ab");
        // non-ASCII UTF-8 byte (signed <= 0) forces quotes
        assert_eq!(quote_java_identifier("ré", "\""), "\"ré\"");
    }
}
