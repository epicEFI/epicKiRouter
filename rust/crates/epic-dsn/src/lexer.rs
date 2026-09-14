//! Hand-written port of the JFlex-generated Specctra DSN tokenizer
//! (`io.specctra.parser.SpecctraDsnStreamReader`).
//!
//! The Java class is a 1,862-line generated scanner (~1,150 lines of packed
//! DFA tables). What is ported here are its **semantics**, not its DFA: the
//! eight lexical states (`SpecctraDsnStreamReader.java:29-37`), the token
//! kinds (String / Integer / Double / Keyword / open+close bracket / EOF),
//! and the buffer-level helpers `nextString` / `nextStringList` /
//! `nextDouble` (`:1739-1840`).
//!
//! Every rule below is jar-captured (jshell sessions against
//! `build/libs/freerouting-current-executable.jar`, 2026-09-12) or read off
//! the generated action switch; the `#[test]`s cite the session that pinned
//! each value.
//!
//! # Token classification summary (all jar-pinned)
//!
//! - Whitespace is `{space, \t, \n, \r}`; `(` / `)` are bracket tokens and
//!   reset the lexical state to [`LexicalState::YyInitial`].
//! - From `YyInitial`, a word (chars other than whitespace/brackets) is
//!   looked up in the keyword table ([`Keyword::from_bytes`]); non-keywords
//!   scan as [`Token::Str`]. A word starting with a digit scans as an
//!   integer (`[+-]?[0-9]+`) or double (`[+-]?[0-9]+(.[0-9]+)?([eE][+-]?[0-9]+)?`,
//!   also `[0-9]+[eE][+-]?[0-9]+`) via maximal munch of that grammar;
//!   leftovers continue as separate tokens (`1.5x` → `Double(1.5)`,
//!   `Str("x")`; `3.` → `Int(3)`, `Str(".")`; `.5` → `Str(".5")` because the
//!   double grammar requires a leading digit).
//! - From [`LexicalState::Name`] *nothing* is a keyword and numbers scan as
//!   strings (`(net 5)` → `Str("5")`) — T29. A single string/number token
//!   resets the state to `YyInitial`.
//! - From [`LexicalState::LayerName`] only `pcb` and `signal` remain
//!   keywords (`(path signal 42)` → `KW:signal`, `(path via 42)` →
//!   `STR:"via"`) — that is how the special `Shape.getLayer` layer names
//!   survive.
//! - `"` / `'` start quoted strings (content keeps whitespace, brackets,
//!   newlines; the other quote char is content); a closing quote yields one
//!   `Token::Str`. Unterminated quotes swallow the rest of the input and
//!   produce `Token::Eof` (jar: `(net "abc 1)` → EOF). Exception: in
//!   [`LexicalState::IgnoreQuote`] a quote is an ordinary word character
//!   (see that state).
//! - Integer tokens are `Integer.valueOf(yytext())`; overflow throws in
//!   Java (`2147483648` → `NumberFormatException` out of `nextToken`) and
//!   is represented as [`Token::Error`] here. Double tokens are
//!   `Double.valueOf(yytext())`; overflow yields infinity (`1.5e400` →
//!   `DBL:Infinity`, jar-captured).
//!
//! # Known-parallel-quirk notes
//!
//! - Java's `nextString` bounds its scan by the 16 MB scanner buffer, not
//!   by end-of-input, so a call at end-of-input appends NUL padding (16 MB
//!   explosion, observed when probing). Real call sites only call it
//!   between tokens. This port stops at end-of-input instead.
//! - A sign character not followed by a digit at word-start (`+` in
//!   `+.5`) is warned about and skipped by the Java scanner (jar:
//!   "Non-ansi character '+'"). The warning goes to FRLogger only — not to
//!   the parse warnings list — so this port skips such characters silently.
//!   A lone `-` behaves identically (jar `/tmp/epic-corner-probe.jsh`
//!   `minus-word`: `(x -x)` → the `-` is skipped and `x` scans).

use crate::keyword::Keyword;

/// The eight JFlex lexical states (`SpecctraDsnStreamReader.java:29-37`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexicalState {
    /// JFlex `YYINITIAL` (0) — the initial state; full keyword table.
    YyInitial,
    /// JFlex `STRING1` (1) — inside a double-quoted string.
    String1,
    /// JFlex `STRING2` (2) — inside a single-quoted string.
    String2,
    /// JFlex `NAME` (3) — the next word is a name, never a keyword.
    Name,
    /// JFlex `LAYER_NAME` (4) — layer names; only `pcb`/`signal` stay
    /// keywords.
    LayerName,
    /// JFlex `COMPONENT_NAME` (5) — no scanner rule enters this state (no
    /// keyword action selects it in the jar); kept for state-number parity.
    ComponentName,
    /// JFlex `SPEC_CHAR` (6) — no keyword ACTION selects it, but
    /// `Network.readNetPins` enters it by hand (`yybegin(SPEC_CHAR)`) to
    /// overread the `-` between the component and pin halves of a
    /// `R1-1` pin token (`Network.java:219`). In this state only one
    /// scanner rule exists: a lone `-` scans as the one-char String `-`
    /// and the state STAYS `SPEC_CHAR`; every other non-whitespace byte
    /// (brackets included) logs "Non-ansi character ..." (FRLogger-only)
    /// and scans as `null` — the byte is consumed and the state stays
    /// (jar /tmp/epic-t8-scanner captures).
    SpecChar,
    /// JFlex `IGNORE_QUOTE` (7) — entered by the `string_quote` keyword.
    /// There the configured quote character is an ordinary word character:
    /// a token starting with it is the quote plus the maximal run of
    /// non-whitespace non-bracket bytes, VERBATIM — embedded quotes
    /// included (jar /tmp/epic-review-probe.jsh: `'a'b` → ONE
    /// `STR:"'a'b"`, `"a b"` → `STR:""a"` + `STR:"b""`) — and the state
    /// resets after that one word like any other string token (jar
    /// /tmp/epic-iq-probe.jsh: keywords and digits also scan as plain
    /// strings in this state).
    IgnoreQuote,
}

/// A scanned token. Mirrors the object kinds Java's `nextToken()` returns
/// (`Keyword`/`Integer`/`Double`/`String` flyweights compared by `==`).
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// A recognized keyword (jar: keyword flyweight).
    Keyword(Keyword),
    /// An identifier / name / quoted string (jar: `String`).
    Str(Box<str>),
    /// An integer literal (jar: `Integer.valueOf(yytext())`).
    Int(i32),
    /// A double literal (jar: `Double.valueOf(yytext())`).
    Double(f64),
    /// `(` (jar: `Keyword.OPEN_BRACKET`).
    Open,
    /// `)` (jar: `Keyword.CLOSED_BRACKET`).
    Close,
    /// End of input (jar: `null`).
    Eof,
    /// Java throws instead of returning a token: produced when an integer
    /// token overflows `i32` (`Integer.valueOf("2147483648")` throws
    /// `NumberFormatException` out of `nextToken`; jar-captured). Callers
    /// transliterate Java's catch sites: `skip_scope` maps this to `false`,
    /// scope readers let it abort the parse.
    Error(Box<str>),
}

/// The tokenizer. Scans bytes (Java scans chars from a platform-charset
/// `Reader`; the DSN grammar is ASCII and quoted strings are passed
/// through byte-wise, so bytes preserve UTF-8 content).
pub struct Scanner<'a> {
    input: &'a [u8],
    cursor: usize,
    state: LexicalState,
    /// Scratch buffer for quoted-string content (Java `stringBuffer`).
    string_buf: Vec<u8>,
    /// Java `scopeIdentifier` (`SpecctraDsnStreamReader.java:545`, default
    /// `""`): the label the wiring readers stamp with the last-read net /
    /// padstack name and the warning strings interpolate. Java declares it
    /// `static` (shared across scanner instances); a per-scanner field is
    /// behaviorally identical for the one-scanner-per-parse read path.
    scope_identifier: String,
}

impl<'a> Scanner<'a> {
    pub fn new(input: &'a [u8]) -> Scanner<'a> {
        Scanner {
            input,
            cursor: 0,
            state: LexicalState::YyInitial,
            string_buf: Vec::new(),
            scope_identifier: String::new(),
        }
    }

    /// Java `getScopeIdentifier()` (`:861-863`).
    pub fn scope_identifier(&self) -> &str {
        &self.scope_identifier
    }

    /// Java `setScopeIdentifier(String)` (`:865-868`).
    pub fn set_scope_identifier(&mut self, identifier: &str) {
        self.scope_identifier = identifier.to_string();
    }

    /// The current lexical state (Java `yystate()`).
    pub fn lexical_state(&self) -> LexicalState {
        self.state
    }

    /// Switch the lexical state (Java `yybegin(int)`); used by the reader's
    /// header hand-scan (`DsnReader.java:96`) and by [`skip_scope`].
    pub fn set_lexical_state(&mut self, state: LexicalState) {
        self.state = state;
    }

    /// Returns the next token, mirroring Java `nextToken()`.
    ///
    /// The JFlex scanner loops within one `nextToken` call when an action
    /// only switches state without returning (quote-open, non-ANSI skip);
    /// the `loop` here reproduces that.
    pub fn next_token(&mut self) -> Token {
        loop {
            while self.cursor < self.input.len()
                && matches!(self.input[self.cursor], b' ' | b'\t' | b'\n' | b'\r')
            {
                self.cursor += 1;
            }
            let Some(&b) = self.input.get(self.cursor) else {
                return Token::Eof;
            };

            // SPEC_CHAR has no keyword table: one byte per call — the `-`
            // scans as the String "-", anything else as Java `null` (the
            // "Non-ansi character" warn is FRLogger-only). Brackets are NOT
            // special here (the jar eats them, state unchanged) — so this
            // arm MUST sit before the bracket check.
            if self.state == LexicalState::SpecChar {
                self.cursor += 1;
                return if b == b'-' {
                    Token::Str("-".into())
                } else {
                    Token::Eof
                };
            }

            // Brackets are recognized in every lexical state and reset to
            // the initial state (jar: `(net (x))` -> `OPEN(st0)`).
            if b == b'(' {
                self.cursor += 1;
                self.state = LexicalState::YyInitial;
                return Token::Open;
            }
            if b == b')' {
                self.cursor += 1;
                self.state = LexicalState::YyInitial;
                return Token::Close;
            }

            // A quote starts a quoted string EXCEPT in IGNORE_QUOTE, where
            // the configured quote character is an ordinary word character
            // (jar /tmp/epic-review-probe.jsh: `'a` scans as the verbatim
            // word `'a`, `'a'b` as ONE token, `"a b"` as two words) — that
            // case falls through to the word scan below.
            if matches!(b, b'"' | b'\'') && self.state != LexicalState::IgnoreQuote {
                return self.scan_quoted(b);
            }

            // Numbers and the non-ANSI sign skip only apply from the
            // initial state; in NAME / LAYER_NAME / IGNORE_QUOTE everything
            // scans as words (jar: `(net 5)` -> STR, `(path 42 1)` ->
            // STR:"42", `(string_quote 5)` -> STR:"5").
            if self.state == LexicalState::YyInitial {
                if matches!(b, b'+' | b'-') {
                    let next_is_digit = self
                        .input
                        .get(self.cursor + 1)
                        .is_some_and(u8::is_ascii_digit);
                    if b == b'-' && next_is_digit {
                        return self.scan_number();
                    }
                    // Java logs "Non-ansi character '+'/'-' found at
                    // position ..." (FRLogger.warn) and skips the character
                    // (jar: `(x +7)` -> INT:7, `(x -.5)` -> STR:".5"); the
                    // warning is FRLogger-only, so the skip is silent here.
                    self.cursor += 1;
                    continue;
                }
                if b.is_ascii_digit() {
                    return self.scan_number();
                }
            }

            return self.scan_word();
        }
    }

    /// Quoted-string scan (JFlex STRING1/STRING2 states, entered and exited
    /// within this one call). Content keeps whitespace, brackets, newlines
    /// and the other quote character; `"\` + char appends both characters
    /// and closes the string only when char is the quote (jar esc-probe:
    /// `"a\"b"` -> `STR:"a\""` + `STR:"b""`, `"a\\b"` -> `STR:"a\\b"`).
    /// An unterminated string swallows the rest of the input and yields no
    /// token (jar: `(net "abc 1)` -> EOF, content discarded).
    fn scan_quoted(&mut self, quote: u8) -> Token {
        self.cursor += 1; // opening quote
        self.state = if quote == b'"' {
            LexicalState::String1
        } else {
            LexicalState::String2
        };
        self.string_buf.clear();
        loop {
            let Some(&b) = self.input.get(self.cursor) else {
                return Token::Eof;
            };
            if b == quote {
                self.cursor += 1;
                self.state = LexicalState::YyInitial;
                return Token::Str(
                    String::from_utf8_lossy(&self.string_buf)
                        .into_owned()
                        .into_boxed_str(),
                );
            }
            if b == b'\\' {
                self.string_buf.push(b);
                self.cursor += 1;
                if let Some(&d) = self.input.get(self.cursor) {
                    self.string_buf.push(d);
                    self.cursor += 1;
                    if d == quote {
                        self.state = LexicalState::YyInitial;
                        return Token::Str(
                            String::from_utf8_lossy(&self.string_buf)
                                .into_owned()
                                .into_boxed_str(),
                        );
                    }
                }
                continue;
            }
            self.string_buf.push(b);
            self.cursor += 1;
        }
    }

    /// Word scan: one run of non-whitespace non-bracket bytes. Quote
    /// characters are content inside a word (jar: `b"` and `h'i` scan as
    /// single tokens); keyword recognition depends on the state — the full
    /// table from [`LexicalState::YyInitial`], only `pcb`/`signal` from
    /// [`LexicalState::LayerName`], nothing otherwise.
    fn scan_word(&mut self) -> Token {
        let start = self.cursor;
        while self.cursor < self.input.len()
            && !matches!(
                self.input[self.cursor],
                b' ' | b'\t' | b'\n' | b'\r' | b'(' | b')'
            )
        {
            self.cursor += 1;
        }
        let text = &self.input[start..self.cursor];
        match self.state {
            LexicalState::YyInitial => {
                if let Some(kw) = Keyword::from_bytes(text) {
                    // the post-state is keyed by the SPELLING (JFlex
                    // actions): the `wire_keepout` alias keeps YYINITIAL
                    self.state = Keyword::scanned_lexical_state(text, kw);
                    return Token::Keyword(kw);
                }
            }
            LexicalState::LayerName => {
                if let Some(kw) = Keyword::from_bytes(text)
                    && matches!(kw, Keyword::Pcb | Keyword::Signal)
                {
                    self.state = Keyword::scanned_lexical_state(text, kw);
                    return Token::Keyword(kw);
                }
            }
            _ => {}
        }
        self.state = LexicalState::YyInitial;
        Token::Str(String::from_utf8_lossy(text).into_owned().into_boxed_str())
    }

    /// Number scan from the initial state: maximal munch of
    /// `-[0-9]+([.][0-9]+)?([eE][+-]?[0-9]+)?` with backtracking to the
    /// longest complete match (jar: `3.` -> INT:3 + STR:".", `1e` ->
    /// INT:1 + STR:"e", `1e-` -> INT:1 + STR:"e-", `1.5x` -> DBL:1.5 +
    /// STR:"x"). A dot needs a following digit and an exponent needs a
    /// (signable) digit, or they are not consumed.
    fn scan_number(&mut self) -> Token {
        const fn is_digit(b: u8) -> bool {
            b.is_ascii_digit()
        }
        let len = self.input.len();
        let start = self.cursor;
        let mut end = start;
        if self.input[end] == b'-' {
            end += 1;
        }
        while end < len && is_digit(self.input[end]) {
            end += 1;
        }
        let mut is_double = false;
        if end + 1 < len && self.input[end] == b'.' && is_digit(self.input[end + 1]) {
            is_double = true;
            end += 1;
            while end < len && is_digit(self.input[end]) {
                end += 1;
            }
        }
        if end < len && matches!(self.input[end], b'e' | b'E') {
            let mut exp_end = end + 1;
            if exp_end < len && matches!(self.input[exp_end], b'+' | b'-') {
                exp_end += 1;
            }
            if exp_end < len && is_digit(self.input[exp_end]) {
                is_double = true;
                end = exp_end;
                while end < len && is_digit(self.input[end]) {
                    end += 1;
                }
            }
        }
        let text = &self.input[start..end];
        self.cursor = end;
        // from_utf8_lossy is lossless here: the scanned grammar is ASCII.
        let text = String::from_utf8_lossy(text);
        if is_double {
            // Rust's f64 parser is correctly rounded like Double.valueOf;
            // overflow yields infinity (jar: `1.5e400` -> DBL:Infinity).
            // The scanned grammar guarantees a parseable f64.
            Token::Double(
                text.parse::<f64>()
                    .expect("number grammar guarantees a valid f64"),
            )
        } else {
            match text.parse::<i32>() {
                Ok(value) => Token::Int(value),
                // Java: Integer.valueOf throws NumberFormatException out of
                // nextToken (jar: `2147483648`); the error text travels in
                // the token instead of an exception.
                Err(_) => Token::Error(text.into_owned().into_boxed_str()),
            }
        }
    }

    /// Port of `nextString()` (`SpecctraDsnStreamReader.java:1739-1796`):
    /// buffer-level read of a name/quoted string, skipping leading
    /// `{backspace, space}` (plus `{newline, CR}` when `ignore_newline`, and
    /// `leading`), stopping at `{backspace, newline, CR, space, (, )}`
    /// (or `{newline, CR, "}` in quoted form), stepping the cursor back one
    /// byte when the last consumed byte is a bracket (T27).
    ///
    /// A name is read with `next_string(false, b' ')`; resets the lexical
    /// state to [`LexicalState::YyInitial`] (Java line 1784). `leading` is
    /// a single byte; Java's `char` separator only round-trips for ASCII
    /// (the DSN grammar is ASCII).
    pub fn next_string_with(&mut self, ignore_newline: bool, leading: u8) -> String {
        self.string_buf.clear();
        let mut i = 0usize;
        let len = self.input.len();

        // Java `stringSkipTrailing` = {backspace, space},
        // `stringSkipTrailingNewLines` = {backspace, newline, CR, space},
        // plus the `leading` character (SpecctraDsnStreamReader.java:593-596,
        // :1743-1757).
        let is_skippable =
            |b: u8| b == leading || b == 8 || b == 32 || (ignore_newline && (b == 10 || b == 13));
        while self.cursor + i < len && is_skippable(self.input[self.cursor + i]) {
            i += 1;
        }

        let quoted = self.cursor + i < len && self.input[self.cursor + i] == 34;
        let mut skip_last_char = false;
        if quoted {
            // `stringStopAtQuotes` = {newline, CR, double-quote}; both quote
            // bytes are consumed around the content (:1761-1764, :1776).
            i += 1;
            skip_last_char = true;
        }
        // `stringStopAt` = {backspace, newline, CR, space, '(', ')'} plus
        // the leading char; the stop byte itself is never consumed, so the
        // next nextToken re-scans it (jar probe2 T27/T27b).
        let is_stop = |b: u8| {
            quoted && (b == 10 || b == 13 || b == 34)
                || !quoted
                    && (b == 8
                        || b == 10
                        || b == 13
                        || b == 32
                        || b == 40
                        || b == 41
                        || b == leading)
        };
        while self.cursor + i < len && !is_stop(self.input[self.cursor + i]) {
            self.string_buf.push(self.input[self.cursor + i]);
            i += 1;
        }
        if skip_last_char {
            i += 1;
        }

        if i > 0 {
            // Java advances zzMarkedPos by i and resets the lexical state
            // (:1780-1784); the zzStartRead/zzCurrentPos adjustments are
            // scanner-internal read positions with no effect on the next
            // nextToken. The bracket step-back (:1786-1792) is transliterated
            // for parity even though the stop sets above make it hard to
            // reach. Java bounds all of this by its 16 MB buffer (reading NUL
            // padding past the input); this port stops at end of input.
            self.cursor = (self.cursor + i).min(len);
            self.state = LexicalState::YyInitial;
            if self.cursor > 0 && matches!(self.input[self.cursor - 1], 40 | 41) {
                self.cursor -= 1;
            }
        }

        String::from_utf8_lossy(&self.string_buf).into_owned()
    }

    /// Java `nextString()` — the common `ignoreNewline=false, leading=' '`
    /// form used by nearly all call sites.
    pub fn next_string(&mut self) -> String {
        self.next_string_with(false, b' ')
    }

    /// Port of `nextStringList` (`SpecctraDsnStreamReader.java:1798-1827`)
    /// with the caller-supplied separator character: reads strings until an
    /// empty one; an empty *first* element is dropped (T28, the KiCad 8
    /// netlist workaround). The separator doubles as a skip-leading
    /// character and a stop character, which is how the KiCad
    /// clearance-class pair splits (jar /tmp/epic-review-probe2.jsh:
    /// `default-default` with separator `-` → `["default", "default"]`).
    /// `Rule.readClearanceRule` calls the Java original with
    /// `DsnFile.CLASS_CLEARANCE_SEPARATOR = '-'` (`Rule.java:277`,
    /// parser/DsnFile.java:20); the common space-separated form is
    /// [`Scanner::next_string_list`]. Java also rewinds
    /// `zzStartRead`/`zzCurrentPos` by one here (`:1823-1824`) — cosmetic
    /// position bookkeeping that does not move the resume cursor
    /// (`zzMarkedPos`), so the next `next_token` still sees the `)`
    /// terminator (jar: probe T28, next token after the list is `Close`).
    pub fn next_string_list_with(&mut self, separator: u8) -> Vec<String> {
        let mut result = Vec::new();
        // Every string list must have at least one item, but the first item
        // can be empty; that one is ignored (the KiCad 8 netlist workaround,
        // SpecctraDsnStreamReader.java:1805-1812).
        let first = self.next_string_with(true, separator);
        if !first.is_empty() {
            result.push(first);
        }
        loop {
            let next = self.next_string_with(true, separator);
            if next.is_empty() {
                break;
            }
            result.push(next);
        }
        // Java then rewinds zzStartRead/zzCurrentPos to zzMarkedPos - 1
        // (:1823-1824) — cosmetic bookkeeping that does not move the resume
        // cursor (zzMarkedPos), so the closing bracket is still the next
        // token (jar probe2 T28).
        result
    }

    /// Java `nextStringList()` — the space-separated form used by nearly
    /// all call sites.
    pub fn next_string_list(&mut self) -> Vec<String> {
        self.next_string_list_with(b' ')
    }

    /// Port of `nextDouble()` (`SpecctraDsnStreamReader.java:1829-1840`):
    /// reads a string and parses it with `NumberFormat.getInstance(Locale.US)`
    /// lenient prefix semantics, `None` on failure (T26; Java returns null
    /// on ParseException).
    pub fn next_double(&mut self) -> Option<f64> {
        let s = self.next_string();
        parse_number_format_us(s.as_bytes())
    }

    /// Port of `nextClosingBracket()`
    /// (`SpecctraDsnStreamReader.java:1842-1861`): consumes one token and
    /// requires a closing bracket. Returns `false` on end of input (Java
    /// warns "unexpected end of file") or on any other token (Java warns
    /// "expected closed bracket is missing"; the offending token is
    /// consumed either way, and both warnings go to FRLogger only). An
    /// error token (integer overflow, where Java's `nextToken` throws out
    /// of this method) also yields `false` — callers treat the result as
    /// fatal for the rule either way. Call sites: `Rule.readWidthRule`
    /// (`Rule.java:112`) and `readClearanceRule` (`:280`, `:289`).
    ///
    /// Java divergence at end of input: with the resume point already at
    /// end of input, Java's buffer refill CRASHES
    /// (`ArrayIndexOutOfBoundsException` in `zzRefill`,
    /// /tmp/epic-review-probe2.jsh) instead of reaching the null check —
    /// the same 16 MB-buffer family as `nextString` at EOF. This port
    /// models the clean `false` the source intends.
    pub fn next_closing_bracket(&mut self) -> bool {
        matches!(self.next_token(), Token::Close)
    }
}

/// Lenient `java.text.NumberFormat.getInstance(Locale.US).parse(...)` prefix
/// semantics over a decimal string (T26; every value below jar-captured):
///
/// - optional `-` sign; `+` is NOT accepted as a sign (`parse("+7")` throws)
/// - integer digits, optional fraction (`parse(".5")` → 0.5, `parse("3.")` → 3)
/// - the exponent is NOT consumed (`parse("1e5")` → 1, `parse("1.5e2")` → 1.5)
/// - no digits at all → `None` (`parse("abc")`, `parse("")` throw)
fn parse_number_format_us(s: &[u8]) -> Option<f64> {
    let mut i = 0usize;
    if i < s.len() && s[i] == b'-' {
        i += 1;
    }
    // Integer digits with lenient comma grouping — commas are pure noise,
    // anywhere in the run (jar: "1,234"->1234, "12,34"->1234, ",123"->123,
    // "1,,2"->12, "12,345,678"->12345678), but whitespace is not skippable
    // (" 7" and "- 7" throw).
    while i < s.len() && (s[i].is_ascii_digit() || s[i] == b',') {
        i += 1;
    }
    // Optional fraction; the dot needs no digits after it ("3." -> 3.0) and
    // a second dot terminates the parse ("1.5.5" -> 1.5, ".5.5" -> 0.5).
    // A leading dot without integer digits is fine (".5" -> 0.5, "-.5" ->
    // -0.5), but "-" / "." / "-." have no digits at all and throw.
    if i < s.len() && s[i] == b'.' {
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
    }
    if !s[..i].iter().any(u8::is_ascii_digit) {
        return None;
    }
    // The exponent is NOT part of the prefix grammar ("1e5" -> 1, "1.5e2"
    // -> 1.5); parse stops at the first char that cannot continue
    // ("1.5x" -> 1.5, "7 " -> 7).
    let mut recon = String::with_capacity(i);
    for &b in &s[..i] {
        if b != b',' {
            recon.push(b as char);
        }
    }
    recon.parse::<f64>().ok()
}

/// Strict `Double.parseDouble` semantics (T26's second parser; used by
/// `Package.readRotation`, `Package.java:339`): the whole string must be a
/// decimal number — `parseDouble("1.5e2")` → 150.0 while lenient parsing
/// yields 1.5 — with one optional trailing `d`/`D`/`f`/`F` suffix (jar
/// /tmp/epic-review-probe.jsh: `5l`/`5L` throw NumberFormatException,
/// `5e2d` → 500.0).
///
/// Known divergences, jar /tmp/epic-review-probe.jsh `parseDouble edges`:
/// Java also accepts the exact spellings `Infinity`/`-Infinity`/`NaN`
/// (which Rust accepts too) and hex-float literals (`0x1.8p1` → 3.0,
/// rejected by Rust), while it THROWS on Rust's lowercase `nan`/`infinity`
/// spellings. None of these forms can appear in a DSN rotation value, so
/// Rust's native accept set is kept (see the pinning test).
pub fn parse_double_strict(s: &str) -> Option<f64> {
    // Java accepts one trailing float-type suffix d/D/f/F only (`5l`
    // throws); Rust's parser rejects any suffix, so strip it first.
    let stripped = s.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(s);
    stripped.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyword::skip_scope;

    fn tokens(input: &str) -> Vec<Token> {
        let mut scanner = Scanner::new(input.as_bytes());
        let mut out = Vec::new();
        for _ in 0..40 {
            let token = scanner.next_token();
            let done = token == Token::Eof;
            out.push(token);
            if done {
                break;
            }
        }
        out
    }

    /// Scanner advanced past a fixed number of leading tokens, for probing
    /// `next_string`/`next_double` in the same call order Java readers use
    /// (they never call those at end of input).
    fn scanner_after(input: &str, n: usize) -> Scanner<'_> {
        let mut scanner = Scanner::new(input.as_bytes());
        for _ in 0..n {
            scanner.next_token();
        }
        scanner
    }

    fn str_token(s: &str) -> Token {
        Token::Str(Box::from(s))
    }

    // ------------------------------------------------------------------
    // Token streams, state by state
    // ------------------------------------------------------------------

    /// Jar session `/tmp/epic-lexer-probe.jsh`, `toks net-gnd-1`:
    /// `OPEN st0, KW:net st3, STR:"GND" st0, INT:1 st0, CLOSE st0, EOF`.
    #[test]
    fn net_gnd_1_token_stream() {
        let mut scanner = Scanner::new(b"(net GND 1)");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Net));
        assert_eq!(scanner.lexical_state(), LexicalState::Name);
        assert_eq!(scanner.next_token(), str_token("GND"));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        assert_eq!(scanner.next_token(), Token::Int(1));
        assert_eq!(scanner.next_token(), Token::Close);
        assert_eq!(scanner.next_token(), Token::Eof);
    }

    /// T29: jar sessions `via-initial` (`KW:via st=3`), `net-via-name-state`
    /// (`STR:"via" st=0`), keyword-sweep `(image via 42)` (`STR:"via"`):
    /// `via` is a keyword from the initial state but a plain string in the
    /// NAME state.
    #[test]
    fn via_keyword_from_initial_is_string_in_name_state() {
        assert_eq!(tokens("(via)")[1], Token::Keyword(Keyword::Via));
        assert_eq!(
            tokens("(net via 1)"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("via"),
                Token::Int(1),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(tokens("(image via 42)")[2], str_token("via"));
    }

    /// Jar keyword-sweep `(net 5)` → `STR:"5"`, `(net (x))` →
    /// `OPEN(st0)`: in the NAME state numbers scan as strings and `(`
    /// resets to the initial state.
    #[test]
    fn name_state_numbers_are_strings_and_open_resets() {
        assert_eq!(
            tokens("(net 5)"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("5"),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("(net (x))"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                Token::Open,
                str_token("x"),
                Token::Close,
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar sessions `layer-name-state` and keyword-sweep `(path ...)`
    /// probes: layer names are strings; in LAYER_NAME state only `pcb` and
    /// `signal` stay keywords; `pcb`/`signal` from the NAME state are
    /// strings (`/tmp/epic-final-probe.jsh`).
    #[test]
    fn layer_name_state_semantics() {
        // (layer top (type signal))
        assert_eq!(
            tokens("(layer top (type signal))"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Layer),
                str_token("top"),
                Token::Open,
                Token::Keyword(Keyword::Type),
                Token::Keyword(Keyword::Signal),
                Token::Close,
                Token::Close,
                Token::Eof,
            ]
        );
        // LAYER_NAME state entered by the shape keywords
        assert_eq!(tokens("(path via 42)")[2], str_token("via"));
        assert_eq!(
            tokens("(path signal 42)")[2],
            Token::Keyword(Keyword::Signal)
        );
        assert_eq!(tokens("(path pcb 42)")[2], Token::Keyword(Keyword::Pcb));
        // jar /tmp/epic-word-probe.jsh path-circle-kw: STR:"polygon" etc.
        assert_eq!(tokens("(path circle 42)")[2], str_token("circle"));
        // NAME state: even pcb/signal are plain strings
        assert_eq!(tokens("(net pcb 42)")[2], str_token("pcb"));
        assert_eq!(tokens("(net signal 42)")[2], str_token("signal"));
    }

    /// Jar `/tmp/epic-lexer-probe.jsh` `skip-scope-unknown`: `(unit mm)`
    /// tokenizes with no UNIT keyword — `unit` is a plain string (T24's
    /// lexical side).
    #[test]
    fn unit_scope_is_not_a_keyword() {
        assert_eq!(
            tokens("(pcb X (unit mm) (resolution um 10))"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Pcb),
                str_token("X"),
                Token::Open,
                str_token("unit"),
                str_token("mm"),
                Token::Close,
                Token::Open,
                Token::Keyword(Keyword::Resolution),
                str_token("um"),
                Token::Int(10),
                Token::Close,
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar: `string_quote` switches to the IGNORE_QUOTE state (st=7) and
    /// the lone quote char directly before `)` scans as a one-char word
    /// (the verbatim quote-run ends at the bracket) resetting to the
    /// initial state (`toks string-quote-ignore` + custom-quote probe in
    /// `/tmp/epic-num-probe.jsh`; the quote-run rule itself is pinned by
    /// `ignore_quote_quote_starts_verbatim_word` below).
    #[test]
    fn string_quote_enters_ignore_quote_state() {
        let mut scanner = Scanner::new(b"(parser (string_quote '))");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Parser));
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::StringQuote));
        assert_eq!(scanner.lexical_state(), LexicalState::IgnoreQuote);
        assert_eq!(scanner.next_token(), str_token("'"));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        assert_eq!(scanner.next_token(), Token::Close);
        assert_eq!(scanner.next_token(), Token::Close);
    }

    // ------------------------------------------------------------------
    // next_string (T27)
    // ------------------------------------------------------------------

    /// Jar `/tmp/epic-lexer-probe2.jsh`: after consuming `(net`,
    /// `nextString` on `(net GND)` yields "GND" and the following token is
    /// CLOSE (the terminator is re-scannable), on `(net GND 1)` it is
    /// `INT:1`.
    #[test]
    fn next_string_stops_and_lets_terminator_rescan() {
        let mut scanner = scanner_after("(net GND)", 2);
        assert_eq!(scanner.next_string(), "GND");
        assert_eq!(scanner.next_token(), Token::Close);

        let mut scanner = scanner_after("(net GND 1)", 2);
        assert_eq!(scanner.next_string(), "GND");
        assert_eq!(scanner.next_token(), Token::Int(1));
    }

    /// Jar probe2 T27c: quoted content via `nextString` — quotes stripped,
    /// spaces kept.
    #[test]
    fn next_string_quoted_form() {
        let mut scanner = scanner_after("(net \"hello world\" 1)", 2);
        assert_eq!(scanner.next_string(), "hello world");
        assert_eq!(scanner.next_token(), Token::Int(1));
    }

    /// Jar probe2 T27d: `ignoreNewline=true` skips leading newlines between
    /// class members.
    #[test]
    fn next_string_ignores_newlines_when_asked() {
        let input = "(class \"kicad_default\"\n  GND\n  VCC\n)";
        let mut scanner = scanner_after(input, 3);
        assert_eq!(scanner.next_string_with(true, b' '), "GND");
        assert_eq!(scanner.next_token(), str_token("VCC"));
    }

    /// Java `nextString` line 1780: an empty read (`i == 0`, cursor already
    /// at a terminator) moves nothing and leaves the state untouched
    /// (code-pinned; the jar probes exercise the `i > 0` path only).
    #[test]
    fn next_string_empty_read_moves_nothing() {
        let mut scanner = scanner_after("()", 1);
        assert_eq!(scanner.next_string(), "");
        assert_eq!(scanner.next_token(), Token::Close);
    }

    // ------------------------------------------------------------------
    // SPEC_CHAR (Network.readNetPins overread state)
    // ------------------------------------------------------------------

    /// Jar `/tmp/epic-t8-scanner` (probe `two-pins`, input `(R1-1))`):
    /// after `nextString(true, '-')` reads "R1", `yybegin(SPEC_CHAR)` +
    /// `nextToken` yields the one-char String "-" and the state STAYS
    /// SPEC_CHAR; the following `nextString(true)` reads the pin "1" and
    /// resets to YYINITIAL, so the scope's closing bracket still scans as
    /// CLOSE. The empty-pin form `(R1-))` diverges: the empty pin read
    /// leaves SPEC_CHAR active (Java resets the state only for `i > 0`),
    /// the post-loop `nextToken` eats the first `)` as Non-ansi/null, and
    /// readNetPins reports the missing end of file (`null`) — the pin
    /// entry `(R1, "")` is still added (jar probe `empty-pin`).
    #[test]
    fn spec_char_hyphen_overread_and_pin_scan() {
        let mut scanner = scanner_after("(R1-1))", 1); // '(' consumed
        assert_eq!(scanner.next_string_with(true, b'-'), "R1");
        scanner.set_lexical_state(LexicalState::SpecChar);
        assert_eq!(scanner.next_token(), str_token("-"));
        assert_eq!(scanner.lexical_state(), LexicalState::SpecChar);
        assert_eq!(scanner.next_string_with(true, b' '), "1");
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        assert_eq!(scanner.next_token(), Token::Close);
    }

    /// Jar probe `empty-pin` continuation: in SPEC_CHAR a `)` is NOT a
    /// bracket — it is consumed as Non-ansi and scans `null` (Java), the
    /// state stays SPEC_CHAR, and EOF scans `null` too. This is the arm
    /// that must sit BEFORE the bracket check (the discriminating form:
    /// a bracket-first check would return CLOSE and "resynchronize" the
    /// scope walk where Java desynchronizes).
    #[test]
    fn spec_char_brackets_and_letters_scan_null_and_stay() {
        let mut scanner = scanner_after("x)a", 0);
        scanner.set_lexical_state(LexicalState::SpecChar);
        assert_eq!(scanner.next_token(), Token::Eof);
        assert_eq!(scanner.lexical_state(), LexicalState::SpecChar);
        assert_eq!(scanner.next_token(), Token::Eof);
        assert_eq!(scanner.lexical_state(), LexicalState::SpecChar);
        assert_eq!(scanner.next_token(), Token::Eof);
        // and the input is fully consumed: after resetting the state the
        // scanner is at end of input.
        scanner.set_lexical_state(LexicalState::YyInitial);
        assert_eq!(scanner.next_token(), Token::Eof);
    }

    // ------------------------------------------------------------------
    // next_string_list (T28)
    // ------------------------------------------------------------------

    /// Jar probe2 T28: the KiCad 8 shape `(class "kicad_default" "" GND
    /// VCC)` reads `["GND", "VCC"]` — the empty first element is dropped —
    /// and the next token is the closing bracket.
    #[test]
    fn next_string_list_drops_empty_first_element() {
        let mut scanner = scanner_after("(class \"kicad_default\" \"\" GND VCC)", 3);
        assert_eq!(scanner.next_string_list(), vec!["GND", "VCC"]);
        assert_eq!(scanner.next_token(), Token::Close);

        let mut scanner = scanner_after("(class \"kicad_default\" GND VCC)", 3);
        assert_eq!(scanner.next_string_list(), vec!["GND", "VCC"]);
        assert_eq!(scanner.next_token(), Token::Close);
    }

    // ------------------------------------------------------------------
    // next_double vs strict parse (T26)
    // ------------------------------------------------------------------

    /// T26, jar `/tmp/epic-num-probe.jsh` + the NumberFormat block of
    /// `/tmp/epic-lexer-probe.jsh`: `nextDouble` is lenient
    /// `NumberFormat(Locale.US)` prefix parsing (exponent NOT consumed),
    /// `Double.parseDouble` (readRotation, `Package.java:339`) is strict.
    #[test]
    fn next_double_is_lenient_parse_double_is_strict() {
        // lenient nextDouble, scanner call order (consume "( x" first)
        let mut scanner = scanner_after("(x 1e5)", 2);
        assert_eq!(scanner.next_double(), Some(1.0));
        let mut scanner = scanner_after("(x 1.5e2)", 2);
        assert_eq!(scanner.next_double(), Some(1.5));
        let mut scanner = scanner_after("(x .5)", 2);
        assert_eq!(scanner.next_double(), Some(0.5));
        let mut scanner = scanner_after("(x 3.)", 2);
        assert_eq!(scanner.next_double(), Some(3.0));
        let mut scanner = scanner_after("(x abc)", 2);
        assert_eq!(scanner.next_double(), None);

        // strict parseDouble
        assert_eq!(parse_double_strict("1.5e2"), Some(150.0));
        assert_eq!(parse_double_strict("1.5x"), None);
        assert_eq!(parse_double_strict(".5"), Some(0.5));
        assert_eq!(parse_double_strict("abc"), None);
        assert_eq!(parse_double_strict(""), None);
    }

    /// Jar NumberFormat block: `parse("+7")` throws (no `+` sign prefix)
    /// while `parse("-7")` is -7; `parse("")` throws.
    #[test]
    fn number_format_sign_rules() {
        assert_eq!(parse_number_format_us(b"-7"), Some(-7.0));
        assert_eq!(parse_number_format_us(b"+7"), None);
        assert_eq!(parse_number_format_us(b""), None);
        assert_eq!(parse_number_format_us(b"abc"), None);
    }

    // ------------------------------------------------------------------
    // Numeric tokens
    // ------------------------------------------------------------------

    /// Jar `/tmp/epic-lexer-probe.jsh` `toks doubles` (first tokens) and
    /// `/tmp/epic-lexer-probe2.jsh` tok lines: exponent doubles,
    /// leading-dot strings, sign handling, `3.` split.
    #[test]
    fn numeric_token_grammar() {
        assert_eq!(
            tokens("(x 1.5e2 .5 1.5x)"),
            vec![
                Token::Open,
                str_token("x"),
                Token::Double(150.0),
                str_token(".5"),
                Token::Double(1.5),
                str_token("x"),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("(x -7 +7 -1.5e2 +.5 3.)"),
            vec![
                Token::Open,
                str_token("x"),
                Token::Int(-7),
                Token::Int(7),
                Token::Double(-150.0),
                // "+.5": jar warns "Non-ansi character '+'" and skips the
                // plus; ".5" scans as a string (leading digit required).
                str_token(".5"),
                Token::Int(3),
                str_token("."),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("(x 2.5e-3 1e5 0.0)"),
            vec![
                Token::Open,
                str_token("x"),
                Token::Double(0.0025),
                Token::Double(100_000.0),
                Token::Double(0.0),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(tokens("(x -1.5)")[2], Token::Double(-1.5));
        // jar esc-probe huge-double: DBL:Infinity
        assert_eq!(tokens("(x 1.5e400)")[2], Token::Double(f64::INFINITY));
    }

    /// Jar probe `toks doubles`: the integer token `2147483648` makes Java
    /// `nextToken` THROW `NumberFormatException: For input string:
    /// "2147483648"`. Here it surfaces as [`Token::Error`]; the scanner
    /// then continues after the digits (Java aborts the scan instead).
    #[test]
    fn integer_overflow_is_an_error_token() {
        let mut scanner = Scanner::new(b"(x 2147483648)");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), str_token("x"));
        assert!(matches!(scanner.next_token(), Token::Error(_)));
        assert_eq!(scanner.next_token(), Token::Close);
        assert_eq!(scanner.next_token(), Token::Eof);
    }

    // ------------------------------------------------------------------
    // Words and quoted strings
    // ------------------------------------------------------------------

    /// Jar `/tmp/epic-word-probe.jsh` word-chars / plus-minus-mid:
    /// `-`, `:`, `[`, `]`, `_`, `.`, `'`, `+` inside a word all scan as one
    /// string token.
    #[test]
    fn word_characters() {
        assert_eq!(
            tokens("(x U1-1 a:b c[1] d_e f.g h'i)"),
            vec![
                Token::Open,
                str_token("x"),
                str_token("U1-1"),
                str_token("a:b"),
                str_token("c[1]"),
                str_token("d_e"),
                str_token("f.g"),
                str_token("h'i"),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("(x a+b c-d)"),
            vec![
                Token::Open,
                str_token("x"),
                str_token("a+b"),
                str_token("c-d"),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar probe `toks quoted` and `quoted-with-parens`: brackets and
    /// whitespace are content inside quotes.
    #[test]
    fn quoted_strings_keep_brackets_and_spaces() {
        assert_eq!(
            tokens("(net \"a(b)c\" 2)"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("a(b)c"),
                Token::Int(2),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar word-probe newline-in-quote: a newline inside a quoted string is
    /// content, not a terminator.
    #[test]
    fn quoted_strings_keep_newlines() {
        assert_eq!(
            tokens("(net \"ab\ncd\" 1)"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("ab\ncd"),
                Token::Int(1),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar quote-probe unterminated-quote: `(net "abc 1)` — the quote
    /// swallows the rest of the input and the scanner reports EOF (Java
    /// returns null).
    #[test]
    fn unterminated_quote_swallows_rest_of_input() {
        assert_eq!(
            tokens("(net \"abc 1)"),
            vec![Token::Open, Token::Keyword(Keyword::Net), Token::Eof]
        );
    }

    /// Jar esc-probe escape-quote: inside a quoted string `\"` appends the
    /// backslash-plus-quote and CLOSES the string (`STR:"a\"`), and the
    /// following `b"` scans as one word token (`STR:"b""` — quote chars are
    /// ordinary word-continuation characters).
    #[test]
    fn escaped_quote_inside_string() {
        assert_eq!(
            tokens(r#"(net "a\"b" 1)"#),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("a\\\""),
                str_token("b\""),
                Token::Int(1),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar esc-probe escape-backslash: a backslash pair is kept verbatim.
    #[test]
    fn escaped_backslash_inside_string() {
        assert_eq!(
            tokens(r#"(net "a\\b" 1)"#),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("a\\\\b"),
                Token::Int(1),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Single-quoted strings scan like double-quoted ones (jar
    /// quote-probe custom-quote-net: `(net 'hello world' 1)` →
    /// `STR:"hello world"` even without any string_quote configuration).
    #[test]
    fn single_quoted_strings() {
        assert_eq!(
            tokens("(net 'hello world' 1)"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("hello world"),
                Token::Int(1),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    // ------------------------------------------------------------------
    // path alias, EOF, skip_scope
    // ------------------------------------------------------------------

    /// Jar keyword-sweep `path-alias`: `(wire (path F.Cu 125 1 2 3 4))` —
    /// `path` scans as the POLYGON_PATH keyword with LAYER_NAME post-state,
    /// the layer name then resets to the initial state.
    #[test]
    fn path_alias_token_stream() {
        assert_eq!(
            tokens("(wire (path F.Cu 125 1 2 3 4))"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Wire),
                Token::Open,
                Token::Keyword(Keyword::PolygonPath),
                str_token("F.Cu"),
                Token::Int(125),
                Token::Int(1),
                Token::Int(2),
                Token::Int(3),
                Token::Int(4),
                Token::Close,
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// EOF edges (jar probes `only-close`, `empty-input`): a bare close is
    /// a token; end of input yields Eof repeatedly.
    #[test]
    fn eof_edges() {
        assert_eq!(tokens(")"), vec![Token::Close, Token::Eof]);
        assert_eq!(tokens(""), vec![Token::Eof]);
        let mut scanner = Scanner::new(b"");
        assert_eq!(scanner.next_token(), Token::Eof);
        assert_eq!(scanner.next_token(), Token::Eof);
    }

    /// `set_lexical_state` mirrors the reader's forced NAME state
    /// (`DsnReader.java:96`): a keyword-lookalike design name scans as a
    /// string.
    #[test]
    fn forced_name_state_makes_keywords_strings() {
        let mut scanner = Scanner::new(b"(pcb via");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Pcb));
        scanner.set_lexical_state(LexicalState::Name);
        assert_eq!(scanner.next_token(), str_token("via"));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
    }

    /// `skip_scope` (see keyword.rs tests for the jar citations) leaves the
    /// cursor at the token after the skipped scope and scans the huge
    /// integer inside it as a string (NAME state), unlike the throwing
    /// initial-state scan. Call order mirrors `ScopeKeyword.readScope`: the
    /// reader has consumed `(unit` as tokens before dispatching the skip.
    #[test]
    fn skip_scope_after_unknown_scope() {
        let mut scanner = Scanner::new(b"(unit mm 2147483648 (x (y))) (resolution um 10)");
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), str_token("unit"));
        assert!(skip_scope(&mut scanner));
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Resolution));
    }

    // ------------------------------------------------------------------
    // Corner cases pinned in /tmp/epic-corner-probe.jsh (2026-09-12)
    // ------------------------------------------------------------------

    /// Jar corner-probe `minus-word` / `minus-dot-five` / `plus-word`: a
    /// `+` or `-` at word start NOT followed by a digit warns "Non-ansi
    /// character ..." in Java and is skipped (the warning goes to FRLogger
    /// only, so the skip is silent here); the rest scans normally.
    #[test]
    fn sign_not_followed_by_digit_is_skipped() {
        assert_eq!(
            tokens("(x -x)"),
            vec![
                Token::Open,
                str_token("x"),
                str_token("x"),
                Token::Close,
                Token::Eof
            ]
        );
        assert_eq!(
            tokens("(x -.5)"),
            vec![
                Token::Open,
                str_token("x"),
                str_token(".5"),
                Token::Close,
                Token::Eof
            ]
        );
        assert_eq!(
            tokens("(x +x)"),
            vec![
                Token::Open,
                str_token("x"),
                str_token("x"),
                Token::Close,
                Token::Eof
            ]
        );
    }

    /// Jar corner-probe `name-state-minus-int`: `-7` in the NAME state is
    /// one plain string (no number rules outside the initial state).
    #[test]
    fn name_state_negative_number_is_a_string() {
        assert_eq!(
            tokens("(net -7 1)"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Net),
                str_token("-7"),
                Token::Int(1),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar corner-probe `dangling-e` / `dangling-e-dot` / `dangling-e-sign`:
    /// the DFA backtracks to the longest complete number when the exponent
    /// is dangling; the leftovers scan as words (`e-` is one word because
    /// the sign is a word-continuation character).
    #[test]
    fn dangling_exponent_backtracks() {
        assert_eq!(
            tokens("(x 1e 2)"),
            vec![
                Token::Open,
                str_token("x"),
                Token::Int(1),
                str_token("e"),
                Token::Int(2),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("(x 1.5e 2)"),
            vec![
                Token::Open,
                str_token("x"),
                Token::Double(1.5),
                str_token("e"),
                Token::Int(2),
                Token::Close,
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("(x 1e- 2)"),
            vec![
                Token::Open,
                str_token("x"),
                Token::Int(1),
                str_token("e-"),
                Token::Int(2),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar corner-probe `two-dots`: maximal munch stops at the second dot;
    /// the trailing `.5` scans as a word (leading digit required for
    /// numbers).
    #[test]
    fn double_dot_splits() {
        assert_eq!(
            tokens("(x 1.5.5)"),
            vec![
                Token::Open,
                str_token("x"),
                Token::Double(1.5),
                str_token(".5"),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar corner-probe `layer-name-digit`: digits in the LAYER_NAME state
    /// are plain strings (layer names), not numbers.
    #[test]
    fn layer_name_digits_are_strings() {
        assert_eq!(
            tokens("(path 42 1)"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::PolygonPath),
                str_token("42"),
                Token::Int(1),
                Token::Close,
                Token::Eof,
            ]
        );
    }

    /// Jar `/tmp/epic-iq-probe.jsh` `iq-keyword-word` / `iq-digit` /
    /// `iq-paren`: in the IGNORE_QUOTE state every word scans as a plain
    /// string (keywords and digits included — `net` → STR, `5` → STR:"5"),
    /// and the quote char directly before a bracket is a one-char word
    /// (the quote-run stops at the bracket).
    #[test]
    fn ignore_quote_state_words_are_plain_strings() {
        let mut scanner = Scanner::new(b"(parser (string_quote net))");
        for expected in [
            Token::Open,
            Token::Keyword(Keyword::Parser),
            Token::Open,
            Token::Keyword(Keyword::StringQuote),
        ] {
            assert_eq!(scanner.next_token(), expected);
        }
        assert_eq!(scanner.lexical_state(), LexicalState::IgnoreQuote);
        assert_eq!(scanner.next_token(), str_token("net"));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);

        let mut scanner = Scanner::new(b"(parser (string_quote 5))");
        for _ in 0..4 {
            scanner.next_token();
        }
        assert_eq!(scanner.next_token(), str_token("5"));
        assert_eq!(scanner.next_token(), Token::Close);

        let mut scanner = Scanner::new(b"(parser (string_quote '('))");
        for _ in 0..4 {
            scanner.next_token();
        }
        assert_eq!(scanner.next_token(), str_token("'"));
        assert_eq!(scanner.next_token(), Token::Open);
    }

    /// NumberFormat(Locale.US) lenient prefix edges (jar corner-probe
    /// `NumberFormat parse edges` block + /tmp/epic-nf-probe.jsh): loose
    /// comma grouping anywhere ("12,34" → 1234, ",123" → 123, "1,,2" → 12),
    /// leading-dot fractions ("-.5" → -0.5, ".5.5" → 0.5), prefix parse
    /// ("1.5x" → 1.5, "1.5.5" → 1.5, "7 " → 7, "1.5," → 1.5), but NO
    /// leading whitespace (" 7" throws), no "--7" / "- " / "-." (zero
    /// digits), and no digits at all ("." throws).
    #[test]
    fn number_format_edges() {
        assert_eq!(parse_number_format_us(b"1,234"), Some(1234.0));
        assert_eq!(parse_number_format_us(b"12,34"), Some(1234.0));
        assert_eq!(parse_number_format_us(b"1,23"), Some(123.0));
        assert_eq!(parse_number_format_us(b"12,345,678"), Some(12_345_678.0));
        assert_eq!(parse_number_format_us(b"1,234.5"), Some(1234.5));
        assert_eq!(parse_number_format_us(b",123"), Some(123.0));
        assert_eq!(parse_number_format_us(b"1,,2"), Some(12.0));
        assert_eq!(parse_number_format_us(b"-.5"), Some(-0.5));
        assert_eq!(parse_number_format_us(b".5"), Some(0.5));
        assert_eq!(parse_number_format_us(b".5.5"), Some(0.5));
        assert_eq!(parse_number_format_us(b"1.5x"), Some(1.5));
        assert_eq!(parse_number_format_us(b"1.5.5"), Some(1.5));
        assert_eq!(parse_number_format_us(b"1.5,"), Some(1.5));
        assert_eq!(parse_number_format_us(b"7 "), Some(7.0));
        assert_eq!(parse_number_format_us(b"1e"), Some(1.0));
        assert_eq!(parse_number_format_us(b" 7"), None);
        assert_eq!(parse_number_format_us(b"- 7"), None);
        assert_eq!(parse_number_format_us(b"--7"), None);
        assert_eq!(parse_number_format_us(b"-."), None);
        assert_eq!(parse_number_format_us(b"."), None);
        assert_eq!(parse_number_format_us(b".,"), None);
        assert_eq!(parse_number_format_us(b","), None);
        assert_eq!(parse_number_format_us(b""), None);
        assert_eq!(parse_number_format_us(b"abc"), None);
    }

    // ------------------------------------------------------------------
    // Review-fix pins, session /tmp/epic-review-probe.jsh (2026-09-12)
    // ------------------------------------------------------------------

    /// Jar /tmp/epic-review-probe.jsh `iq-quote-run` etc.: in the
    /// IGNORE_QUOTE state a quote character does NOT start a quoted string —
    /// it is an ordinary word character. A token starting with a quote is
    /// the quote plus the maximal run of non-whitespace non-bracket bytes,
    /// verbatim (embedded quotes included), and the state resets to the
    /// initial state afterwards.
    #[test]
    fn ignore_quote_quote_starts_verbatim_word() {
        // quote + run up to the closing bracket
        let mut scanner = Scanner::new(b"(parser (string_quote 'a))");
        for expected in [
            Token::Open,
            Token::Keyword(Keyword::Parser),
            Token::Open,
            Token::Keyword(Keyword::StringQuote),
        ] {
            assert_eq!(scanner.next_token(), expected);
        }
        assert_eq!(scanner.lexical_state(), LexicalState::IgnoreQuote);
        assert_eq!(scanner.next_token(), str_token("'a"));
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);
        assert_eq!(scanner.next_token(), Token::Close);
        assert_eq!(scanner.next_token(), Token::Close);

        // embedded quotes are verbatim content; ONE token (jar probe
        // `iq-quote-run-embedded` printed 6 tokens: ... STR:"'a'b" CLOSE;
        // the second CLOSE closes the (parser scope)
        assert_eq!(
            tokens("(parser (string_quote 'a'b))"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Parser),
                Token::Open,
                Token::Keyword(Keyword::StringQuote),
                str_token("'a'b"),
                Token::Close,
                Token::Close,
                Token::Eof,
            ]
        );

        // whitespace ends the quote-run; the second fragment starts fresh
        // (jar probe `iq-quote-run-space` printed 7 tokens: ... STR:""a"
        // STR:"b"" CLOSE; the second CLOSE closes the (parser scope)
        assert_eq!(
            tokens(r#"(parser (string_quote "a b"))"#),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Parser),
                Token::Open,
                Token::Keyword(Keyword::StringQuote),
                str_token("\"a"),
                str_token("b\""),
                Token::Close,
                Token::Close,
                Token::Eof,
            ]
        );

        // at end of input the run just ends (words do not swallow input)
        assert_eq!(
            tokens("(parser (string_quote 'a"),
            vec![
                Token::Open,
                Token::Keyword(Keyword::Parser),
                Token::Open,
                Token::Keyword(Keyword::StringQuote),
                str_token("'a"),
                Token::Eof,
            ]
        );

        // digits inside the run are verbatim, not a number token
        assert_eq!(tokens("(parser (string_quote '5))")[4], str_token("'5"));
    }

    /// Strict-double suffix set (jar /tmp/epic-review-probe.jsh `parseDouble
    /// edges`): only d/D/f/F are accepted suffixes; l/L throw
    /// NumberFormatException in Java. A suffix after an exponent works too
    /// ("5e2d" -> 500.0).
    #[test]
    fn parse_double_strict_suffix_set() {
        assert_eq!(parse_double_strict("5f"), Some(5.0));
        assert_eq!(parse_double_strict("5F"), Some(5.0));
        assert_eq!(parse_double_strict("5d"), Some(5.0));
        assert_eq!(parse_double_strict("5D"), Some(5.0));
        assert_eq!(parse_double_strict("5e2d"), Some(500.0));
        assert_eq!(parse_double_strict("5l"), None);
        assert_eq!(parse_double_strict("5L"), None);
    }

    /// Known Rust/Java divergences in the strict double grammar, jar
    /// /tmp/epic-review-probe.jsh `parseDouble edges`: Java accepts only
    /// the exact spellings `Infinity`/`-Infinity`/`NaN` plus hex-float
    /// literals (`0x1.8p1` → 3.0, rejected by Rust), and THROWS on the
    /// lowercase spellings Rust accepts. None of these forms can appear in
    /// a DSN rotation value, so the port keeps Rust's native accept/reject
    /// set and pins the delta here so a future "fix" does not silently
    /// flip it.
    #[test]
    fn parse_double_strict_inf_nan_divergence_is_documented() {
        // "inf": Java THROWS; the suffix strip eats the trailing f
        // ("inf" -> "in") so Rust rejects too — coincidental agreement.
        assert_eq!(parse_double_strict("inf"), None);
        // "nan": Java THROWS (only "NaN" parses). Rust: accepted.
        // (NaN is never equal to itself, so assert on the predicate.)
        assert!(parse_double_strict("nan").is_some_and(|v| v.is_nan()));
        // Hex float: Java parseDouble("0x1.8p1") == 3.0. Rust: rejected.
        assert_eq!(parse_double_strict("0x1.8p1"), None);
        // The exact Java spellings agree (jar: they parse).
        assert_eq!(parse_double_strict("Infinity"), Some(f64::INFINITY));
        assert!(parse_double_strict("NaN").is_some_and(|v| v.is_nan()));
    }

    /// `next_string_list_with('-')` as `Rule.readClearanceRule` uses it
    /// (`Rule.java:277`, separator `DsnFile.CLASS_CLEARANCE_SEPARATOR = '-'
    ///`, parser/DsnFile.java:20). Jar /tmp/epic-review-probe2.jsh: the
    /// separator acts as skip-leading AND stop char, splitting the KiCad
    /// clearance class pair; the KiCad 8 empty-first drop works with any
    /// separator; afterwards the two closing brackets are still tokens.
    #[test]
    fn next_string_list_with_dash_separator() {
        let mut scanner = scanner_after("(clearance 10 (type default-default))", 5);
        assert_eq!(
            scanner.next_string_list_with(b'-'),
            vec!["default", "default"]
        );
        assert!(scanner.next_closing_bracket());
        assert!(scanner.next_closing_bracket());

        let mut scanner = scanner_after("(clearance 10 (type \"\" A-B))", 5);
        assert_eq!(scanner.next_string_list_with(b'-'), vec!["A", "B"]);
    }

    /// `next_closing_bracket` (port of `nextClosingBracket`,
    /// `SpecctraDsnStreamReader.java:1842-1861`): consumes ONE token and
    /// requires the closing bracket; returns false on any other token or on
    /// end of input, consuming the offending token either way. Warnings go
    /// to FRLogger only.
    ///
    /// Java divergence at EOF: with the resume point at end of input, Java's
    /// buffer refill CRASHES (`ArrayIndexOutOfBoundsException` in zzRefill,
    /// /tmp/epic-review-probe2.jsh) instead of reaching the null check; the
    /// port returns the clean `false` the source intends.
    #[test]
    fn next_closing_bracket_semantics() {
        // Happy path (jar /tmp/epic-review-probe2.jsh `ncb happy`): consume
        // `(width` as tokens, nextDouble reads the value buffer-level, the
        // closing bracket is consumed and true returned.
        let mut scanner = scanner_after("(width 10)", 2);
        assert_eq!(scanner.next_double(), Some(10.0));
        assert!(scanner.next_closing_bracket());
        assert_eq!(scanner.lexical_state(), LexicalState::YyInitial);

        // Wrong token (jar /tmp/epic-review-probe.jsh `ncb wrong-token`,
        // same token flow: nextDouble reads "width" and fails, then ncb
        // consumes INT:10 and returns false, so the NEXT token is INT:5).
        let mut scanner = scanner_after("(width 10 5)", 1);
        assert_eq!(scanner.next_double(), None);
        assert!(!scanner.next_closing_bracket());
        assert_eq!(scanner.next_token(), Token::Int(5));

        // End of input (jar /tmp/epic-review-probe2.jsh `ncb eof`): nextDouble
        // still reads 10.0, then ncb returns false (Java crashes in zzRefill
        // before reaching its null check — documented divergence above).
        let mut scanner = scanner_after("(width 10", 2);
        assert_eq!(scanner.next_double(), Some(10.0));
        assert!(!scanner.next_closing_bracket());
        assert_eq!(scanner.next_token(), Token::Eof);
    }
}
