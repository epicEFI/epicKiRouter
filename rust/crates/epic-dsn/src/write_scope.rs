//! Seed of the SES writer port (Task 13's `write_scope.rs` in the M1b
//! plan): the identifier-quoting rule. It lives in epic-dsn because BOTH
//! consumers need it — the harness parse digest (`dsn_digest`) and the
//! Task 13 SES writer — and the dependency runs harness -> epic-dsn only,
//! so a harness-local copy could never be shared with the writer.
//!
//! Task 13 added the emission core: [`IndentFileWriter`] (the exact
//! `datastructures/IndentFileWriter.java` layout) and
//! [`java_double_to_string`] (the `String.valueOf(double)` /
//! `Double.toString` text, needed for window-hole coordinates the
//! conduction-area writer emits in double precision).

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

/// Java `IndentFileWriter` (`datastructures/IndentFileWriter.java`) over
/// an in-memory `String` instead of a stream: UTF-8 text, `\n` newlines,
/// two-space indent per scope level. `startScope(false)` writes the
/// opening paren WITHOUT a preceding newline (the session root);
/// `endScope` ALWAYS newline-firsts (`:42-50`), so the very first
/// `endScope` of a `startScope(false)` session leaves no stray indent
/// (level 0).
#[derive(Debug, Default)]
pub struct IndentFileWriter {
    buffer: String,
    indent_level: i32,
}

impl IndentFileWriter {
    /// An empty writer at indent level 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The accumulated text (the flushed stream contents).
    pub fn into_string(self) -> String {
        self.buffer
    }

    /// Java `startScope(boolean)` (`:23-34`): optional leading newline,
    /// then `(` and the indent level grows.
    pub fn start_scope_with_newline(&mut self, new_line: bool) {
        if new_line {
            self.new_line();
        }
        self.buffer.push('(');
        self.indent_level += 1;
    }

    /// Java `startScope()` (`:37-39`): always newline-first.
    pub fn start_scope(&mut self) {
        self.start_scope_with_newline(true);
    }

    /// Java `endScope()` (`:42-50`): indent shrinks FIRST, then newline +
    /// `)`.
    pub fn end_scope(&mut self) {
        self.indent_level -= 1;
        self.new_line();
        self.buffer.push(')');
    }

    /// Java `newLine()` (`:53-62`): `\n` plus the current indent.
    pub fn new_line(&mut self) {
        self.buffer.push('\n');
        for _ in 0..self.indent_level {
            self.buffer.push_str("  ");
        }
    }

    /// Java `write(String)` — raw append.
    pub fn write(&mut self, text: &str) {
        self.buffer.push_str(text);
    }
}

/// Java `Double.toString(double)` / `String.valueOf(double)` — JDK 19+
/// shortest-round-trip digits with Java's LAYOUT rules, which differ from
/// Rust's `Display`:
///
/// - plain decimal `ddd.ddd` (at least one fraction digit) iff
///   `1e-3 <= |v| < 1e7`, else scientific `d.dddE±x` (uppercase `E`, no
///   `+` on positive exponents, at least one fraction digit: `1.0E-4`,
///   not `1E-4`);
/// - `NaN`, `Infinity`, `-Infinity`, `0.0`, `-0.0` verbatim.
///
/// The DIGITS come from Rust's `{:e}` (shortest round-trip, same digit
/// sequence as JDK 19+'s Ryu); only the rendering is Java's. Rust `{}`
/// would print `12345678` where Java prints `1.2345678E7` and `0.0001`
/// where Java prints `1.0E-4` — the two threshold rules this replica
/// exists for. The appending core [`java_double_to_string_into`] holds
/// the algorithm; this wrapper is its single-value form.
pub fn java_double_to_string(value: f64) -> String {
    let mut out = String::new();
    java_double_to_string_into(value, &mut out);
    out
}

/// A fixed-capacity byte buffer implementing [`std::fmt::Write`] — the
/// allocation-free backing for the `{magnitude:e}` render below. The
/// shortest-round-trip form of a finite non-zero `f64` carries at most
/// 17 significant digits, so the whole `d.ddddde±xxx` render is bounded
/// by 1 + 1 + 16 + 1 + 1 + 3 = 23 bytes (worst case
/// `1.7976931348623157e-308`); 48 leaves headroom, and `write_str`'s
/// overflow Err is unreachable by construction (the caller asserts the
/// `Ok`).
struct StackFmt {
    buf: [u8; 48],
    len: usize,
}

impl StackFmt {
    const fn new() -> Self {
        StackFmt {
            buf: [0; 48],
            len: 0,
        }
    }

    /// Only ASCII (`{:e}` output: digits, `.`, `e`, `-`) is ever
    /// written, so the UTF-8 check is static.
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len])
            .expect("StackFmt only receives ASCII `{:e}` renders")
    }
}

impl std::fmt::Write for StackFmt {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let Some(rest) = self.buf.get_mut(self.len..) else {
            return Err(std::fmt::Error);
        };
        if s.len() > rest.len() {
            return Err(std::fmt::Error);
        }
        rest[..s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
        Ok(())
    }
}

/// The appending core of [`java_double_to_string`] — identical output,
/// ZERO allocations: the `{magnitude:e}` render lands in a stack buffer
/// ([`StackFmt`]), the digit sequence is extracted into a second stack
/// buffer (shortest round-trip ≤ 17 digits), and every piece is pushed
/// straight into `out`. The M5-T5 slice calls this PER MAZE ELEMENT
/// from the `RAW_SECTION` row builders (`MazeSearchEngine`); the
/// owning [`java_double_to_string`] delegates here, so the SES/DSN
/// output byte-path is unchanged.
pub fn java_double_to_string_into(value: f64, out: &mut String) {
    if value.is_nan() {
        out.push_str("NaN");
        return;
    }
    if value.is_infinite() {
        out.push_str(if value > 0.0 { "Infinity" } else { "-Infinity" });
        return;
    }
    if value == 0.0 {
        out.push_str(if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        });
        return;
    }
    let negative = value < 0.0;
    let magnitude = value.abs();
    // "d.dddde<exp>" — digits without the point, exponent of the FIRST
    // digit (1.xxxx * 10^exp10). Infallible: bound 23 bytes < 48 (the
    // overflow Err the write returns on is unreachable, asserted live
    // like the digit-buffer bound below).
    let mut sci = StackFmt::new();
    assert!(
        std::fmt::write(&mut sci, format_args!("{magnitude:e}")).is_ok(),
        "the {{:e}} render exceeds the 48-byte StackFmt buffer (bound: 23)"
    );
    let scientific = sci.as_str();
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        out.push_str(scientific); // unreachable for finite non-zero
        return;
    };
    // Rust `{:e}` always writes a parseable exponent (any sign lives in
    // the exponent half, never the mantissa), so both the parse and the
    // absence of a `+` prefix on `digits` are invariants, not error paths.
    let exp10: i32 = exponent
        .parse()
        .expect("Rust {:e} exponent is always a parseable i32");
    let mut digit_buf = [0u8; 24];
    let mut ndigits = 0usize;
    for b in mantissa.bytes() {
        if b != b'.' {
            assert!(
                ndigits < digit_buf.len(),
                "shortest-round-trip digits exceed the 24-byte buffer (bound: 17)"
            );
            digit_buf[ndigits] = b;
            ndigits += 1;
        }
    }
    let digits: &str =
        std::str::from_utf8(&digit_buf[..ndigits]).expect("the digit bytes are ASCII");
    if negative {
        out.push('-');
    }
    if (-3..=6).contains(&exp10) {
        // plain decimal, 1e-3 <= |v| < 1e7
        let point_position = exp10 + 1; // digits before the point
        let n = digits.len() as i32;
        if point_position <= 0 {
            out.push_str("0.");
            for _ in 0..-point_position {
                out.push('0');
            }
            out.push_str(digits);
        } else if point_position >= n {
            out.push_str(digits);
            for _ in 0..(point_position - n) {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            let split = point_position as usize;
            out.push_str(&digits[..split]);
            out.push('.');
            out.push_str(&digits[split..]);
        }
    } else {
        // Java scientific: d.dddE<exp> — at least one fraction digit
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        let _ = std::fmt::write(out, format_args!("{exp10}"));
    }
}

/// Java `board.model.structure.Unit.toString()` (`Unit.java:36-38`:
/// `super.toString().toLowerCase()`) — the lowercase enum name the
/// `(resolution <unit> <n>)` SES/DSN scope writes.
pub fn unit_to_dsn_string(unit: crate::state::Unit) -> &'static str {
    match unit {
        crate::state::Unit::Mil => "mil",
        crate::state::Unit::Inch => "in",
        crate::state::Unit::Mm => "mm",
        crate::state::Unit::Um => "um",
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
        // APOSTROPHE pin (plan :284's quote-decision dimensions): `'` is
        // NOT in the reserved set (`SesWriter.java:62`:
        // {"(", ")", " ", ";", "-", "_", "/", "~", "{", "}"}), is ASCII,
        // and is not the string quote — `IdentifierType.write` quotes only
        // on quote-char/reserved/non-ASCII (`IdentifierType.java:24-63`),
        // so `O'Brien` stays UNQUOTED. The counterintuitive case a wrong
        // "quote all punctuation" implementation would flip.
        assert_eq!(quote_java_identifier("O'Brien", "\""), "O'Brien");
        // NON-`"` string quote (anchor-blind rule, the Task-1
        // `(string_quote '` precedent): the quote char itself is STRIPPED
        // from names and used for the wrap, so a hardcode-to-`"`
        // regression flips both rows — `O'Brien` under quote `'` loses its
        // apostrophe, and a reserved-char name wraps in `'…'`, not `"…"`.
        // No tier A+B fixture declares a non-`"` quote, so the goldens
        // cannot catch this class; only these rows do.
        assert_eq!(quote_java_identifier("O'Brien", "'"), "OBrien");
        assert_eq!(quote_java_identifier("a b", "'"), "'a b'");
    }

    /// `IndentFileWriter` layout pin — the exact byte layout of
    /// `datastructures/IndentFileWriter.java`: root `startScope(false)`
    /// opens without a newline, every nested `startScope()`
    /// newline+indent-firsts BEFORE its `(`, and `endScope` decrements
    /// then newline+indent+`)` — so every scope CLOSES on its own line at
    /// the parent indent. The `(place ...)` line demonstrates the OTHER
    /// emission style `SesWriter.writeComponent` actually uses for
    /// inline scopes (`SesWriter.java:167-181`): raw `newLine` +
    /// `"(place "` … `")"` writes, never startScope/endScope — which is
    /// why real SES files show `(place front X Y 0)` closed inline.
    #[test]
    fn indent_file_writer_layout() {
        let mut writer = IndentFileWriter::new();
        writer.start_scope_with_newline(false);
        writer.write("session");
        writer.start_scope(); // placement, level 2
        writer.write("placement");
        writer.start_scope(); // component, level 3
        writer.write("component U1");
        // the writeComponent inline style: newLine + raw "(place ...)"
        writer.new_line();
        writer.write("(place U1 100 100 front 0)");
        writer.end_scope(); // component
        writer.end_scope(); // placement
        writer.end_scope(); // session
        assert_eq!(
            writer.into_string(),
            "(session\n  (placement\n    (component U1\n      (place U1 100 100 front 0)\n    )\n  )\n)"
        );
    }

    /// `endScope` NEVER omits its newline — even at indent 0 straight
    /// after a `startScope(false)` root, the closer is `\n)` (the Java
    /// method unconditionally newline-firsts; a writer that suppressed
    /// the level-0 newline would break every one-line-scope emission).
    #[test]
    fn indent_file_writer_root_close_always_newlines() {
        let mut writer = IndentFileWriter::new();
        writer.start_scope_with_newline(false);
        writer.write("session");
        writer.end_scope();
        assert_eq!(writer.into_string(), "(session\n)");
    }

    /// `java_double_to_string` against known `Double.toString` outputs —
    /// every value's expected text was produced by JDK 25's
    /// `Double.toString`/`String.valueOf(double)` (same algorithm since
    /// JDK 19's Ryu integration). The rows pin BOTH threshold rules
    /// (1e-3 and 1e7), the mandatory-fraction-digit rule (`1.0E-4`, not
    /// `1E-4`; `10.0`, not `10`), the no-`+` positive exponent, and the
    /// shortest-round-trip digit sequence (the 2.15 row: Rust `{}`
    /// would print `2.15` correctly, but `2150000000000000.5` exercises
    /// the plain-range integer+fraction split with more digits than
    /// fit comfortably before the point).
    #[test]
    fn java_double_to_string_pins() {
        assert_eq!(java_double_to_string(0.0), "0.0");
        assert_eq!(java_double_to_string(-0.0), "-0.0");
        assert_eq!(java_double_to_string(f64::NAN), "NaN");
        assert_eq!(java_double_to_string(f64::INFINITY), "Infinity");
        assert_eq!(java_double_to_string(f64::NEG_INFINITY), "-Infinity");
        // plain range [1e-3, 1e7)
        assert_eq!(java_double_to_string(1.0), "1.0");
        assert_eq!(java_double_to_string(10.0), "10.0");
        assert_eq!(java_double_to_string(0.001), "0.001");
        assert_eq!(java_double_to_string(9999999.0), "9999999.0");
        assert_eq!(java_double_to_string(-123.456), "-123.456");
        // just past the thresholds -> scientific with fraction digit
        assert_eq!(java_double_to_string(0.0001), "1.0E-4");
        assert_eq!(java_double_to_string(10000000.0), "1.0E7");
        assert_eq!(java_double_to_string(12345678.0), "1.2345678E7");
        assert_eq!(java_double_to_string(-0.000123456), "-1.23456E-4");
        // shortest round-trip digits carried over from Rust's {:e}
        assert_eq!(java_double_to_string(2.15), "2.15");
        // captured from the jar (jshell /tmp/epic-t13-doubles.jsh): the
        // value is EXACTLY representable (4300000000000001/2), so the
        // 17-digit form is the shortest that round-trips.
        assert_eq!(
            java_double_to_string(2150000000000000.5),
            "2.1500000000000005E15"
        );
        // subnormal territory: digits + deep negative exponent
        assert_eq!(java_double_to_string(1.0e-300), "1.0E-300");
        // trailing-zero-only fraction stays minimal (no padding)
        assert_eq!(java_double_to_string(100.0), "100.0");
    }

    /// The plain-range INTEGER split arm: a value whose shortest digits
    /// are fewer than the integer places needed (`point_position >= n`)
    /// must zero-pad and force `.0` — `1.0E5`-shaped magnitudes inside
    /// the plain range print as `100000.0`, not `1e5` or `100000`.
    #[test]
    fn java_double_to_string_integer_padding() {
        assert_eq!(java_double_to_string(100000.0), "100000.0");
        assert_eq!(java_double_to_string(5000.0), "5000.0");
        // fraction-only-with-leading-zeros arm (point_position <= 0):
        // 1e-3 itself is still in range: "0.001"
        assert_eq!(java_double_to_string(0.001), "0.001");
        // 1.25e-3 = 0.00125
        assert_eq!(java_double_to_string(0.00125), "0.00125");
    }

    /// The M5-T5 appending core: writes INTO the caller's buffer
    /// without clobbering prior content, across ALL render arms
    /// (special values, plain range with the integer-padding and
    /// leading-zero faces, scientific), and the owning
    /// [`java_double_to_string`] is exactly its single-value form (the
    /// delegation contract — a wrapper that stops delegating still
    /// passes the pins above, but this face holds the append-only
    /// behavior the maze row builders rely on).
    #[test]
    fn java_double_to_string_into_appends_without_clobbering() {
        let mut out = String::from("x=");
        java_double_to_string_into(2.15, &mut out);
        assert_eq!(out, "x=2.15");
        java_double_to_string_into(-0.000123456, &mut out);
        assert_eq!(out, "x=2.15-1.23456E-4");
        java_double_to_string_into(100000.0, &mut out);
        assert_eq!(out, "x=2.15-1.23456E-4100000.0");
        java_double_to_string_into(f64::NAN, &mut out);
        assert_eq!(out, "x=2.15-1.23456E-4100000.0NaN");
        let mut fresh = String::new();
        java_double_to_string_into(-0.0, &mut fresh);
        assert_eq!(fresh, "-0.0");
    }

    /// `Unit.toString()` is the lowercase enum name (`Unit.java:36-38`).
    #[test]
    fn unit_to_dsn_string_lowercase_names() {
        assert_eq!(unit_to_dsn_string(crate::state::Unit::Mil), "mil");
        assert_eq!(unit_to_dsn_string(crate::state::Unit::Inch), "in");
        assert_eq!(unit_to_dsn_string(crate::state::Unit::Mm), "mm");
        assert_eq!(unit_to_dsn_string(crate::state::Unit::Um), "um");
    }
}
