//! Java `io.specctra.parser.Network` (`Network.java`, 1479 LOC) plus its
//! sub-readers `NetClass.java` / `Circuit.java` and the rule-side passes
//! they drive (`BoardRules`, `NetClasses`, `KiCadNetClassNames`,
//! `rules.NetClass` setters, `BasicBoard` insertions): the `(network ...)`
//! scope — nets and subnets with pins, via infos, via rules, net classes,
//! class-class clearance pairs, and the board insertion passes
//! (components, pins, package keepouts, outlines, logical parts).
//!
//! Shape: the scope reader parses the whole `(network ...)` body into
//! local IRs ([`DsnNetClass`], [`DsnClassClass`], [`ViaInfoIr`], via-rule
//! name lists), then runs the insertion tail in Java's order — via
//! padstacks, via infos, via rules, net classes, class pairs, components,
//! logical parts. Ordering is LOAD-BEARING end to end; the jar battery
//! pins in `/tmp/epic-t8-probe.out` (net battery), `/tmp/epic-t8-pairbat.out`
//! (clearance pair splits), and `/tmp/epic-t8-loopprobe.out` (the
//! instrumented pair loop) document the observable outcomes.
//!
//! Parity notes the tests lean on:
//!
//! - **Class-scope clearance pairs split at EVERY `_`.** Both
//!   `Network.addClearanceRule` (`:777`) and `addMixedClearanceRule`
//!   (`:651`) use unlimited `String.split("_")` and skip pairs that do
//!   not yield EXACTLY 2 parts, so `slow_kicad_default` splits into
//!   `["slow", "kicad", "default"]` and is INERT (jar
//!   `/tmp/epic-t8-loopprobe.out`: `split=[slow, kicad, default]`, no
//!   classes appended), while `fast_slow` appends `slow-fast` and
//!   `slow-slow`. This is a DIFFERENT splitter from
//!   `Structure.setClearanceRule`'s `split(regex, 2)` (limit 2), which
//!   processes `c__d` as `["c", "_d"]` — Task 5 territory.
//! - **Bare class scopes mis-consume.** `NetClass.readScope` seeds
//!   `prevToken` with the first token AFTER the net list
//!   (`NetClass.java:79`); a class without an inner scope consumes its
//!   own `)` as the seed and the loop then swallows the next scope's
//!   opener+keyword, or — at the end of the network body — the NETWORK's
//!   closing bracket. In a real file the dispatch then breaks on the
//!   PCB's `)` believing the scope ended (tail still runs), and the outer
//!   `ScopeKeyword.readScope` ends cleanly on EOF (`if (nextToken ==
//!   null) return true`) — jar deg battery: RESULT Success, WARNINGS 0.
//!   Ported mechanically; the deg battery pins it (the test harness adds
//!   the pcb close — see `run_board`).
//! - **`insertClassPairs` drains on an alias iterator** (`it2 = it1`,
//!   `Network.java:557`): pairs are `(first, each subsequent)` for the
//!   FIRST class name that resolves; after the drain the outer loop ends
//!   for that class-class scope.
//! - **Net-scope width rules divide in DSN space** — `(int)
//!   Math.round(dsnToBoard(w) / 2)` (`Network.java:1455`) — while CLASS
//!   width rules divide the DSN value first, `dsnToBoard(w / 2)`
//!   (`:485`, `:510`). Same result for exact halves, different rounding
//!   at `.5` boundaries. The net-scope match scan is
//!   `NetClasses.find(hw, tcc, viaRule)` (`NetClasses.java:60-80`):
//!   first class (class 0 included) with equal trace clearance class,
//!   the SAME via-rule OBJECT (`Option<u32>` id here — None matches
//!   None, like Java null == null) and EVERY layer's half width equal.
//! - **`createViaRule` names its rule after the BOARD class** (for the
//!   `kicad_default` alias that is `default`, `Network.java:694`) and
//!   matches `use_via` entries against the via-info PADSTACK names
//!   exactly, so `ViaP.1` matches nothing (empty rule) — jar
//!   `/tmp/epic-t8-probe.out` `VIARULE default []`. Its `attachAllowed`
//!   parameter (`:688`) is dead in the body.
//! - **The tail via-padstack resolution REPLACES the list** and does not
//!   dedup: `[ViaP, ViaQ, ViaSpare] + use_via ViaP.1` resolves to
//!   `[ViaP, ViaQ, ViaSpare, ViaP]` (jar `/tmp/epic-t8-probe.out`
//!   structvia file block). Via infos recorded earlier keep referencing
//!   padstacks the shrunk list no longer contains.
//! - **Tail `viaPadstackNames` seeding** (`Network.java:1273-1280`): a
//!   STRUCTURE `(via ...)` scope seeds the name list — class use_via
//!   entries are APPENDED into it, the classes keep their own lists. With
//!   no structure seed, `viaPadstackNames = n.useVia` is an OBJECT alias
//!   to the FIRST class's own `useVia` — later classes addAll into it,
//!   which `insertNetClass` then reads for `createViaRule` (ported with
//!   the `(owner, merged)` write-back). A non-null seed (either flavor)
//!   makes the tail REPLACE the via list — an empty replace WIPES it (jar
//!   `/tmp/epic-t4c-images.out`). Names resolve case-INSENSITIVELY
//!   against the padstack REGISTRY (`Padstacks.get`, equalsIgnoreCase) —
//!   unlike the via-LIST probe, which is `String.equals`.
//! - **`ViaInfos.add` dedups by name** (`ViaInfos.java:22-28`): the
//!   second `ViaP` info for the structvia 4-padstack via list is
//!   silently dropped, so 4 padstacks yield 3 infos (jar structvia block:
//!   `VIA_PADSTACKS [ViaP ViaQ ViaSpare ViaP]`, `VIAINFO_COUNT 3`).
//! - **Discarded booleans.** `read_net_scope`'s result is dropped by the
//!   dispatch (`:1248`), and `createDefaultViaRule`'s boolean is dropped
//!   at both call sites (`:541`, `:392`) — only a thrown AIOOBE/NPE
//!   kills the parse. `create_default_via_rule` here returns false only
//!   on those exception-equivalent paths, so the death checks are kept;
//!   `insert_logical_parts`' false is genuinely swallowed.
//! - **Known scanner-null substitutions** (Task 5 precedent): Java
//!   `nextString`/`readStringScope` can return null; the pinned Rust
//!   scanner folds those to `""`. The two class-scope arms differ in
//!   Java: `(via_rule ...)` STORES the null unchecked
//!   (`viaRule = DsnFile.readStringScope(scanner)`, `NetClass.java:
//!   100-101`), so Rust stores `Some("")` whose later `via_rule_no`
//!   lookup misses exactly where Java's null skips — no state
//!   difference. For `(clearance_class ...)` a null is PARSE-FATAL in
//!   Java (`if (traceClearanceClass == null) return null`,
//!   `NetClass.java:110-114`); the fold makes that branch unreachable
//!   here — `Some("")` is stored and the lookup silently misses
//!   (accepted Task-5 divergence on malformed input only).

use crate::coordinate_transform::CoordinateTransform;
use crate::keyword::{Keyword, skip_scope};
use crate::lexer::{LexicalState, Scanner, Token};
use crate::scope::rule::{
    LayerRuleIr, LayerRuleOutcome, Rule, read_layer_rule_scope, read_rule_scope,
};
use crate::scope::structure::{
    contains_wire_clearance_pair, read_on_off_scope, read_string_scope, read_via_padstacks,
};
use crate::ses_board::strip_padstack_alias;
use crate::shape::BoardShape;
use crate::sink::{
    AreaIr, BoardSink, ComponentIr, ComponentOutlineIr, FixedStateIr, ImageKeepoutIr, ItemClassIr,
    KeepoutIr, KeepoutKindIr, LogicalPartIr, LogicalPartPinIr, NetIr, PinIr, ViaInfoIr,
    eq_ignore_case,
};
use crate::state::{ComponentLocation, NetId, NetPin, ParseState};
use epic_geometry::int_point::IntPoint;
use epic_geometry::rounding::java_round;

/// Java `parser.NetClass` parse subset (`NetClass.java:31-53` ctor): one
/// `(class ...)` scope of the network body.
#[derive(Debug)]
struct DsnNetClass {
    /// Java `name` — the scope name string.
    name: String,
    /// Java `netList` — the nets between the class name and the first
    /// inner scope (file order).
    net_list: Vec<String>,
    /// Java `rules` — `(rule ...)` scopes; a `Rule.readScope` null is a
    /// parse DEATH here (`rules.addAll(null)` NPE, `NetClass.java:93`),
    /// so the list is total.
    rules: Vec<Rule>,
    /// Java `layerRules` — `(layer_rule ...)` scopes. Java's collection
    /// ALLOWS null entries (`layerRules.add(null)`, `NetClass.java:96`);
    /// the null kills later, during `insert_net_class` — AFTER earlier
    /// rules were applied.
    layer_rules: Vec<Option<LayerRuleIr>>,
    /// Java `useVia` — `(circuit (use_via ...))` lists, file order.
    use_via: Vec<String>,
    /// Java `useLayer` — `(circuit (use_layer ...))` lists.
    use_layer: Vec<String>,
    /// Java `viaRule` — the `(via_rule <name>)` name, null until seen.
    via_rule: Option<String>,
    /// Java `shoveFixed` (default false).
    shove_fixed: bool,
    /// Java `pullTight` (default true).
    pull_tight: bool,
    /// Java `minTraceLength` (default 0; the LAST `(circuit (length))`
    /// wins — the values OVERWRITE, `NetClass.java:103-104`).
    min_trace_length: f64,
    /// Java `maxTraceLength` (default 0).
    max_trace_length: f64,
    /// Java `traceClearanceClass` — the `(clearance_class <name>)` name.
    trace_clearance_class: Option<String>,
}

/// Java `parser.NetClass.ClassClass` (`NetClass.java:201-220`): one
/// `(class_class ...)` scope.
#[derive(Debug)]
struct DsnClassClass {
    /// Java `classNames` — the `(classes ...)` list, file order.
    class_names: Vec<String>,
    /// Java `rules` — total (a `Rule.readScope` null NPEs at read time).
    rules: Vec<Rule>,
    /// Java `layerRules` — null entries allowed, same deferred death as
    /// [`DsnNetClass::layer_rules`].
    layer_rules: Vec<Option<LayerRuleIr>>,
}

/// The outcome of Java `Circuit.readScope` (`Circuit.java:50-91`).
enum CircuitOutcome {
    /// A `ReadScopeResult` (Java returns one after ANY loop completion,
    /// even with nothing parsed).
    Parsed {
        max_length: f64,
        min_length: f64,
        use_via: Vec<String>,
        use_layer: Vec<String>,
    },
    /// Java returned null (EOF mid-scope): the caller keeps the previous
    /// values (`NetClass.java:100-106`).
    Null,
    /// Java THREW into the caller: `useVia.addAll(readViaPadstacks ==
    /// null)` and `Arrays.stream(readStringListScope == null)` NPE
    /// (`Circuit.java:74`, `:76`) — uncaught, parse death.
    Fatal,
}

/// Java `Network.readScope` (`Network.java:1221-1338`): reads the
/// `(network ...)` body and runs the insertion tail. Returns false when
/// Java's dispatch returns false or dies (both fail the parse).
pub fn read_scope(scanner: &mut Scanner, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    // Java has NO entry guard: every board touch inside the dispatched
    // scopes (`board.rules.nets.add`, `board.library.getViaPadstack`, ...)
    // NPEs without a board and fails the parse. The board exists exactly
    // when the structure scope ran, which also materialized the default
    // net class (class 0) and the coordinate transform.
    if state.coordinate_transform.is_none() || state.layer_structure.is_none() {
        return false;
    }
    if sink.net_classes().is_empty() {
        return false;
    }

    let mut classes: Vec<DsnNetClass> = Vec::new();
    let mut class_class_list: Vec<DsnClassClass> = Vec::new();
    let mut via_infos: Vec<ViaInfoIr> = Vec::new();
    let mut via_rules: Vec<Vec<String>> = Vec::new();

    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java: null -> warn + false; IOException -> error + false.
            return false;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::Net) => {
                    // The return value is DISCARDED (`Network.java:1248`):
                    // a failed net scope does not fail the parse.
                    let _ = read_net_scope(scanner, state, sink);
                }
                Token::Keyword(Keyword::Via) => {
                    if let Some(via_info) = read_via_info(scanner, sink) {
                        via_infos.push(via_info);
                    } else {
                        return false;
                    }
                }
                Token::Keyword(Keyword::ViaRule) => match read_via_rule_scope(scanner) {
                    Some(list) => via_rules.push(list),
                    None => return false,
                },
                Token::Keyword(Keyword::Class) => match read_net_class_scope(scanner) {
                    Some(class) => classes.push(class),
                    None => return false,
                },
                Token::Keyword(Keyword::ClassClass) => match read_class_class_scope(scanner) {
                    Some(class_class) => class_class_list.push(class_class),
                    None => return false,
                },
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = Some(next_token);
    }

    // ---- insertion tail (Java order, `Network.java:1287-1337`) ----------

    // Add any vias defined in the net classes to the list of vias to be
    // instantiated (`:1273-1280`). When the STRUCTURE `(via ...)` scope
    // seeded `via_padstack_names`, every class's use_via is APPENDED into
    // that list (the classes keep their own lists). Otherwise
    // `viaPadstackNames = n.useVia` is an object ALIAS to the FIRST
    // class's own list — later classes addAll into it, which
    // `insert_net_class` then reads for `create_via_rule`. The
    // `(owner, merged)` write-back reproduces the alias.
    let mut names: Option<Vec<String>> = None;
    match state.via_padstack_names.take() {
        Some(mut seeded) => {
            for class in &classes {
                seeded.extend(class.use_via.iter().cloned());
            }
            names = Some(seeded);
        }
        None => {
            let mut merged: Option<(usize, Vec<String>)> = None;
            for (idx, class) in classes.iter().enumerate() {
                match &mut merged {
                    Some((_, list)) => list.extend(class.use_via.iter().cloned()),
                    None => merged = Some((idx, class.use_via.clone())),
                }
            }
            if let Some((owner, list)) = merged {
                classes[owner].use_via = list.clone();
                names = Some(list);
            }
        }
    }

    // Set the via padstacks after network parsing (`:1300-1328`): strip
    // the `.N` aliases (`replaceAll("\\.\\d+", "")`), resolve RAW against
    // the padstack registry (case-insensitive), drop misses with a
    // log-only warn, REPLACE the via list. Fires whenever ANY class was
    // seen — an all-empty tail WIPES the list.
    if let Some(names) = &names {
        let mut resolved: Vec<i32> = Vec::new();
        for name in names {
            let cleaned = strip_padstack_alias(name);
            if let Some(padstack_no) = sink.padstack_no(&cleaned) {
                resolved.push(padstack_no);
            }
        }
        sink.set_via_padstacks(resolved);
    }

    insert_via_infos(&via_infos, sink, state.via_at_smd_allowed);
    if !insert_via_rules(&via_rules, sink) {
        return false;
    }
    for class in &classes {
        if !insert_net_class(class, state, sink) {
            return false;
        }
    }
    if !insert_class_pairs(&class_class_list, state, sink) {
        return false;
    }
    insert_components(state, sink);
    // The return value is DISCARDED (`:1336`): only exceptions kill.
    let _ = insert_logical_parts(state, sink);
    true
}

// ---- sub-readers ---------------------------------------------------------

/// Java `NetClass.readScope` (`NetClass.java:56-139`): one `(class ...)`
/// scope. `None` = Java null (the dispatch fails the parse).
fn read_net_class_scope(scanner: &mut Scanner) -> Option<DsnNetClass> {
    scanner.set_lexical_state(LexicalState::Name);
    let class_name = scanner.next_string();
    let net_list = scanner.next_string_list();

    let mut rules: Vec<Rule> = Vec::new();
    let mut layer_rules: Vec<Option<LayerRuleIr>> = Vec::new();
    let mut use_via: Vec<String> = Vec::new();
    let mut use_layer: Vec<String> = Vec::new();
    let mut via_rule: Option<String> = None;
    let mut trace_clearance_class: Option<String> = None;
    let mut pull_tight = true;
    let mut shove_fixed = false;
    let mut min_trace_length = 0.0f64;
    let mut max_trace_length = 0.0f64;

    // The first token after the net list is read OUTSIDE the loop and
    // seeds `prevToken` (`NetClass.java:79-81`). A bare class (no inner
    // scope) therefore consumes its own `)` as the seed and the loop
    // mis-dispatches on the NEXT scope — see the module docs.
    let mut prev_token = scanner.next_token();
    if matches!(prev_token, Token::Error(_)) {
        return None; // IOException -> catch -> null
    }
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Error(_)) {
            return None; // IOException -> catch -> null
        }
        if next_token == Token::Eof {
            // Java: "unexpected end of file" warn + null.
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Token::Open {
            match next_token {
                Token::Keyword(Keyword::Rule) => match read_rule_scope(scanner) {
                    Some(parsed) => rules.extend(parsed),
                    // `rules.addAll(null)` — uncaught NPE, parse death.
                    None => return None,
                },
                Token::Keyword(Keyword::LayerRule) => match read_layer_rule_scope(scanner) {
                    LayerRuleOutcome::Parsed(layer_rule) => layer_rules.push(Some(layer_rule)),
                    // Null entries are STORED (`layerRules.add(null)`);
                    // the death happens in `insert_net_class`.
                    LayerRuleOutcome::Null => layer_rules.push(None),
                    LayerRuleOutcome::Fatal => return None,
                },
                Token::Keyword(Keyword::ViaRule) => {
                    // Java stores the readStringScope result UNCHECKED — a
                    // null becomes viaRule = null (`NetClass.java:100-101`).
                    // The Task-5 fold stores Some(""), whose later
                    // `via_rule_no` lookup misses like Java's null skips.
                    via_rule = Some(read_string_scope(scanner));
                }
                Token::Keyword(Keyword::Circuit) => match read_circuit_scope(scanner) {
                    CircuitOutcome::Parsed {
                        max_length,
                        min_length,
                        use_via: parsed_use_via,
                        use_layer: parsed_use_layer,
                    } => {
                        max_trace_length = max_length;
                        min_trace_length = min_length;
                        use_via.extend(parsed_use_via);
                        use_layer.extend(parsed_use_layer);
                    }
                    CircuitOutcome::Null => {}
                    CircuitOutcome::Fatal => return None,
                },
                Token::Keyword(Keyword::ClearanceClass) => {
                    // Java is PARSE-FATAL on a null here (`if (
                    // traceClearanceClass == null) return null`,
                    // `NetClass.java:110-114`) — unlike `via_rule`, where
                    // the null is stored. The Task-5 fold makes the fatal
                    // branch unreachable; Some("") misses the lookup
                    // (module docs, accepted divergence).
                    trace_clearance_class = Some(read_string_scope(scanner));
                }
                Token::Keyword(Keyword::ShoveFixed) => {
                    shove_fixed = read_on_off_scope(scanner);
                }
                Token::Keyword(Keyword::PullTight) => {
                    pull_tight = read_on_off_scope(scanner);
                }
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = next_token;
    }

    Some(DsnNetClass {
        name: class_name,
        net_list,
        rules,
        layer_rules,
        use_via,
        use_layer,
        via_rule,
        shove_fixed,
        pull_tight,
        min_trace_length,
        max_trace_length,
        trace_clearance_class,
    })
}

/// Java `NetClass.readClassClassScope` (`NetClass.java:146-183`): one
/// `(class_class ...)` scope. NOTE: no unknown-scope arm — an unknown
/// inner scope is NOT skipped, its tokens fall through the dispatch.
fn read_class_class_scope(scanner: &mut Scanner) -> Option<DsnClassClass> {
    let mut class_names: Vec<String> = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut layer_rules: Vec<Option<LayerRuleIr>> = Vec::new();
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java: null -> warn + null; IOException -> catch -> null.
            return None;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::Classes) => {
                    // `Arrays.stream(readStringListScope(scanner))` NPEs
                    // on the null (missing closing bracket).
                    match read_string_list_scope(scanner) {
                        Some(list) => class_names.extend(list),
                        None => return None,
                    }
                }
                Token::Keyword(Keyword::Rule) => match read_rule_scope(scanner) {
                    Some(parsed) => rules.extend(parsed),
                    None => return None, // `rules.addAll(null)` NPE
                },
                Token::Keyword(Keyword::LayerRule) => match read_layer_rule_scope(scanner) {
                    LayerRuleOutcome::Parsed(layer_rule) => layer_rules.push(Some(layer_rule)),
                    LayerRuleOutcome::Null => layer_rules.push(None),
                    LayerRuleOutcome::Fatal => return None,
                },
                // No skip arm (`NetClass.java:160-175`): any other scope
                // is consumed token-by-token by the outer loop.
                _ => {}
            }
        }
        prev_token = Some(next_token);
    }
    Some(DsnClassClass {
        class_names,
        rules,
        layer_rules,
    })
}

/// Java `Circuit.readScope` (`Circuit.java:50-91`).
fn read_circuit_scope(scanner: &mut Scanner) -> CircuitOutcome {
    let mut min_trace_length = 0.0f64;
    let mut max_trace_length = 0.0f64;
    let mut use_via: Vec<String> = Vec::new();
    let mut use_layer: Vec<String> = Vec::new();
    let mut prev_token: Option<Token> = None;
    loop {
        let next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java: null -> warn + null; IOException -> error + null.
            return CircuitOutcome::Null;
        }
        if next_token == Token::Close {
            break;
        }
        if prev_token == Some(Token::Open) {
            match next_token {
                Token::Keyword(Keyword::Length) => {
                    if let Some((max_length, min_length)) = read_length_scope(scanner) {
                        max_trace_length = max_length;
                        min_trace_length = min_length;
                    }
                }
                Token::Keyword(Keyword::UseVia) => match read_via_padstacks(scanner) {
                    Some(list) => use_via.extend(list),
                    // `useVia.addAll(null)` NPE — uncaught, parse death.
                    None => return CircuitOutcome::Fatal,
                },
                Token::Keyword(Keyword::UseLayer) => match read_string_list_scope(scanner) {
                    Some(list) => use_layer.extend(list),
                    // `Arrays.stream(null)` NPE — uncaught, parse death.
                    None => return CircuitOutcome::Fatal,
                },
                _ => {
                    skip_scope(scanner);
                }
            }
        }
        prev_token = Some(next_token);
    }
    CircuitOutcome::Parsed {
        max_length: max_trace_length,
        min_length: min_trace_length,
        use_via,
        use_layer,
    }
}

/// Java `Circuit.readLengthScope` (`Circuit.java:94-127`): exactly two
/// number tokens (double or integer), then a drain loop whose first
/// iteration sees `prevToken` = the SECOND number. Returns
/// `(maxLength, minLength)` — the ctor order flips the array.
fn read_length_scope(scanner: &mut Scanner) -> Option<(f64, f64)> {
    let mut length_arr = [0.0f64; 2];
    let mut next_token = Token::Eof; // Java starts with null
    for slot in length_arr.iter_mut() {
        next_token = scanner.next_token();
        if matches!(next_token, Token::Eof | Token::Error(_)) {
            // Java: null -> warn + null; IOException -> catch -> null.
            return None;
        }
        match next_token {
            Token::Double(value) => *slot = value,
            Token::Int(value) => *slot = f64::from(value),
            _ => {
                // Java: "number expected" warn + null.
                return None;
            }
        }
    }
    // Drain remaining tokens until the closing bracket; inner scopes are
    // skipped when the previous token was an opener.
    let mut prev_token = next_token;
    loop {
        let current_token = scanner.next_token();
        if matches!(current_token, Token::Eof | Token::Error(_)) {
            return None;
        }
        if current_token == Token::Close {
            break;
        }
        if prev_token == Token::Open {
            skip_scope(scanner);
        }
        prev_token = current_token;
    }
    // `new LengthMatchingRule(lengthArr[0], lengthArr[1])` stores
    // maxLength = arr[0], minLength = arr[1].
    Some((length_arr[0], length_arr[1]))
}

/// Java `DsnFile.readStringListScope`: the string list plus its REQUIRED
/// closing bracket; `None` (Java null) when the bracket is missing.
fn read_string_list_scope(scanner: &mut Scanner) -> Option<Vec<String>> {
    let result = scanner.next_string_list();
    if scanner.next_closing_bracket() {
        Some(result)
    } else {
        None
    }
}

/// Java `Network.readViaInfo` (`Network.java:255-322`): one `(via ...)`
/// rule. `None` = Java null (the dispatch fails the parse).
fn read_via_info(scanner: &mut Scanner, sink: &mut dyn BoardSink) -> Option<ViaInfoIr> {
    scanner.set_lexical_state(LexicalState::Name);
    let Token::Str(name) = scanner.next_token() else {
        // Java: "string expected" warn + null; IOException -> catch null.
        return None;
    };
    scanner.set_lexical_state(LexicalState::Name);
    let Token::Str(padstack_name) = scanner.next_token() else {
        return None;
    };
    // The via-list probe is case-SENSITIVE (`getViaPadstack(String)`);
    // the fallback registry query is case-INSENSITIVE and APPENDS the hit
    // to the via list (`addViaPadstack`).
    let padstack_no = match sink.via_padstack_no(&padstack_name) {
        Some(padstack_no) => padstack_no,
        None => match sink.padstack_no(&padstack_name) {
            Some(padstack_no) => {
                sink.append_via_padstack(padstack_no);
                padstack_no
            }
            // Java: "padstack not found" warn + null.
            None => return None,
        },
    };
    scanner.set_lexical_state(LexicalState::Name);
    let Token::Str(clearance_class_name) = scanner.next_token() else {
        return None;
    };
    // A miss means "identical to the default clearance class"
    // (`BoardRules.defaultClearanceClass()` == 1).
    let clearance_class = sink.clearance_class_no(&clearance_class_name).unwrap_or(1);
    let mut attach_smd_allowed = false;
    let next_token = scanner.next_token();
    if next_token != Token::Close {
        if next_token != Token::Keyword(Keyword::Attach) {
            // Java: "Keyword.ATTACH expected" warn + null.
            return None;
        }
        attach_smd_allowed = true;
        if scanner.next_token() != Token::Close {
            // Java: "closing bracket expected" warn + null.
            return None;
        }
    }
    Some(ViaInfoIr {
        name: name.to_string(),
        padstack_no,
        clearance_class,
        attach_smd_allowed,
    })
}

/// Java `Network.readViaRule` (`Network.java:324-345`): strings in NAME
/// state until the closing bracket.
fn read_via_rule_scope(scanner: &mut Scanner) -> Option<Vec<String>> {
    let mut result: Vec<String> = Vec::new();
    loop {
        scanner.set_lexical_state(LexicalState::Name);
        match scanner.next_token() {
            Token::Close => break,
            Token::Str(value) => result.push(value.to_string()),
            Token::Error(_) => return None, // IOException -> catch -> null
            _ => {
                // Java: null or non-string -> warn + null.
                return None;
            }
        }
    }
    Some(result)
}

/// Java `Network.readNetScope` (`Network.java:1340-1478`): one `(net ...)`
/// scope. The RETURN VALUE IS DISCARDED by the dispatch, but the body can
/// still fail the parse through `read_rule_scope`'s null (NPE), which is
/// why the error paths return false here.
fn read_net_scope(scanner: &mut Scanner, state: &mut ParseState, sink: &mut dyn BoardSink) -> bool {
    let net_name = scanner.next_string();
    let next_token = scanner.next_token();
    if matches!(next_token, Token::Eof | Token::Error(_)) {
        // Java: IOException -> catch -> error + false; EOF falls into the
        // loop below and dies at its null check — same outcome.
        return false;
    }
    let mut subnet_number = 1i32;
    let scope_is_empty = next_token == Token::Close;
    if let Token::Int(value) = next_token {
        subnet_number = value;
    }

    let mut pin_order_found = false;
    let mut pin_list: Vec<NetPin> = Vec::new();
    let mut net_rules: Vec<Rule> = Vec::new();
    let mut subnet_pin_lists: Vec<Vec<NetPin>> = Vec::new();

    if !scope_is_empty {
        let mut prev_token = next_token;
        loop {
            let current_token = scanner.next_token();
            if matches!(current_token, Token::Eof | Token::Error(_)) {
                // Java: null -> warn + false; IOException -> catch false.
                return false;
            }
            if current_token == Token::Close {
                break;
            }
            if prev_token == Token::Open {
                match current_token {
                    Token::Keyword(Keyword::Pins) => {
                        if !read_net_pins(scanner, &mut pin_list) {
                            return false;
                        }
                    }
                    Token::Keyword(Keyword::Order) => {
                        pin_order_found = true;
                        if !read_net_pins(scanner, &mut pin_list) {
                            return false;
                        }
                    }
                    Token::Keyword(Keyword::Fromto) => {
                        let mut subnet_pin_list: Vec<NetPin> = Vec::new();
                        if !read_net_pins(scanner, &mut subnet_pin_list) {
                            return false;
                        }
                        // Java stores a TreeSet: sorted + deduplicated.
                        subnet_pin_list.sort();
                        subnet_pin_list.dedup();
                        subnet_pin_lists.push(subnet_pin_list);
                    }
                    Token::Keyword(Keyword::Rule) => match read_rule_scope(scanner) {
                        Some(parsed) => net_rules.extend(parsed),
                        None => return false, // `netRules.addAll(null)` NPE
                    },
                    Token::Keyword(Keyword::LayerRule) => {
                        // Java: "layer_rule not yet implemented" warn +
                        // skipScope — log-only.
                        skip_scope(scanner);
                    }
                    _ => {
                        skip_scope(scanner);
                    }
                }
            }
            prev_token = current_token;
        }
    }

    if subnet_pin_lists.is_empty() {
        if pin_order_found {
            subnet_pin_lists = create_ordered_subnets(&pin_list);
        } else {
            subnet_pin_lists.push(pin_list);
        }
    }

    for current_pin_list in &subnet_pin_lists {
        let net_id = NetId {
            name: net_name.clone(),
            subnet_number,
        };
        if !state.netlist.contains(&net_id) {
            let contains_plane = state
                .layer_structure
                .as_ref()
                .map(|layers| layers.contains_plane(&net_name))
                .unwrap_or(false);
            // Java only touches the board when `addNet` returned a new
            // net (`Network.java:1414-1419`).
            if state.netlist.add_net(net_id.clone()).is_some() {
                sink.append_net(NetIr {
                    name: net_name.clone(),
                    subnet_number,
                    contains_plane,
                    net_class: 0,
                });
            }
        }
        let Some(current_subnet) = state.netlist.get_mut(&net_id) else {
            // Java: "net not found in netlist" warn + false (unreachable:
            // contains/add/get agree).
            return false;
        };
        // Java `setPins` wraps the list in a TreeSet — REPLACE, sorted,
        // deduplicated (the parser Net's pin set is a BTreeSet here).
        current_subnet.pins = current_pin_list.iter().cloned().collect();

        if !net_rules.is_empty() {
            let Some(board_net_no) = sink.net_no_subnet(&net_name, subnet_number) else {
                // Java: "board net not found" warn + false.
                return false;
            };
            let Some(coordinate_transform) = state.coordinate_transform.as_ref() else {
                return false; // unreachable: the entry guard checked this
            };
            for rule in &net_rules {
                if let Rule::Width { value } = rule {
                    // NET-scope division order: dsnToBoard(w) / 2
                    // (`Network.java:1455`) — NOT the class-rule
                    // dsnToBoard(w / 2).
                    let trace_half_width =
                        java_round(coordinate_transform.dsn_to_board_value(*value) / 2.0) as i32;
                    // `NetClasses.find` (`NetClasses.java:60-80`): first
                    // class — class 0 INCLUDED — with equal tcc, the same
                    // via-rule object and EVERY layer's hw equal.
                    let default_tcc = sink.net_classes()[0].trace_clearance_class;
                    let default_via_rule = sink.net_classes()[0].via_rule;
                    let found = sink.net_classes().iter().position(|current_class| {
                        current_class.trace_clearance_class == default_tcc
                            && current_class.via_rule == default_via_rule
                            && current_class
                                .trace_half_widths
                                .iter()
                                .all(|width| *width == trace_half_width)
                    });
                    let net_rule_idx =
                        found.unwrap_or_else(|| sink.append_generated_net_class() as usize);
                    // `netRule.setTraceHalfWidth(traceHalfwidth)` — the
                    // ALL-layers setter.
                    for width in sink.net_classes_mut()[net_rule_idx]
                        .trace_half_widths
                        .iter_mut()
                    {
                        *width = trace_half_width;
                    }
                    sink.nets_mut()[(board_net_no - 1) as usize].net_class = net_rule_idx as i32;
                } else {
                    // Java: "Rule not yet implemented" warn — log-only
                    // (jar net battery: the FAST clearance rule produces
                    // exactly this warning and nothing else).
                }
            }
        }
        subnet_number += 1;
    }
    true
}

/// Java `Network.readNetPins` (`Network.java:212-253`): `A-1 B-2 ...`
/// pairs until an empty component name; the overread hyphen token is
/// discarded. The trailing non-bracket token is a warn-only affair.
fn read_net_pins(scanner: &mut Scanner, pin_list: &mut Vec<NetPin>) -> bool {
    loop {
        let component_name = scanner.next_string_with(true, b'-');
        if component_name.is_empty() {
            break;
        }
        scanner.set_lexical_state(LexicalState::SpecChar);
        let overread = scanner.next_token(); // overread the hyphen
        if matches!(overread, Token::Error(_)) {
            // Java: IOException -> error + false.
            return false;
        }
        let pin_name = scanner.next_string();
        pin_list.push(NetPin {
            component_name,
            pin_name,
        });
    }
    let next_token = scanner.next_token();
    if matches!(next_token, Token::Eof | Token::Error(_)) {
        // Java: null -> "unexpected end of file" + false; IOException ->
        // catch -> false.
        return false;
    }
    if next_token != Token::Close {
        // Java: "expected closed bracket is missing" warn — LOG-ONLY, the
        // parse continues.
    }
    true
}

/// Java `Network.createOrderedSubnets` (`Network.java:193-210`):
/// consecutive pairs of the pin list, each as a sorted, deduplicated set.
fn create_ordered_subnets(pin_list: &[NetPin]) -> Vec<Vec<NetPin>> {
    let mut result: Vec<Vec<NetPin>> = Vec::new();
    if pin_list.is_empty() {
        return result;
    }
    for pair in pin_list.windows(2) {
        let mut subnet = vec![pair[0].clone(), pair[1].clone()];
        subnet.sort();
        subnet.dedup();
        result.push(subnet);
    }
    result
}

// ---- insertion passes ----------------------------------------------------

/// Java `Network.insertViaInfos` (`Network.java:347-357`).
fn insert_via_infos(via_infos: &[ViaInfoIr], sink: &mut dyn BoardSink, attach_allowed: bool) {
    if !via_infos.is_empty() {
        for via_info in via_infos {
            sink.append_via_info(via_info.clone());
        }
    } else {
        // No via infos found; create defaults from the via padstacks for
        // the DEFAULT net class.
        create_default_via_infos(sink, 0, attach_allowed);
    }
}

/// Java `Network.createDefaultViaInfos` (`Network.java:359-377`): one via
/// info per VIA-LIST padstack, named `<padstack>[-<class>]`
/// (`CLASS_CLEARANCE_SEPARATOR` = "-"), clearance class = the class's VIA
/// item, attach = `attachAllowed && padstack.attachAllowed` (Java
/// `Padstack.attachAllowed` IS `is_drillable`, `Padstack.java:57`).
fn create_default_via_infos(sink: &mut dyn BoardSink, class_idx: usize, attach_allowed: bool) {
    let clearance_class_index =
        sink.net_classes()[class_idx].default_item_clearance_classes[ItemClassIr::Via as usize];
    let is_default_class = class_idx == 0;
    let class_name = sink.net_classes()[class_idx].name.clone();
    for padstack_no in sink.via_padstacks().to_vec() {
        let Some(padstack) = sink.padstack(padstack_no) else {
            // Unreachable: the via list only holds resolved numbers.
            continue;
        };
        let via_attach_allowed = attach_allowed && padstack.drillable;
        let via_name = if is_default_class {
            padstack.name.clone()
        } else {
            format!("{}-{}", padstack.name, class_name)
        };
        sink.append_via_info(ViaInfoIr {
            name: via_name,
            padstack_no,
            clearance_class: clearance_class_index,
            attach_smd_allowed: via_attach_allowed,
        });
    }
}

/// Java `Network.insertViaRules` (`Network.java:379-395`). Returns false
/// only on the `create_default_via_rule` exception-equivalent paths.
fn insert_via_rules(via_rules: &[Vec<String>], sink: &mut dyn BoardSink) -> bool {
    let mut rule_found = false;
    for current_list in via_rules {
        if current_list.len() < 2 {
            continue;
        }
        if add_via_rule(current_list, sink) {
            rule_found = true;
        }
    }
    if !rule_found {
        // `createDefaultViaRule(getDefaultNetClass(), "default")` — the
        // boolean is DISCARDED (`:392`); false here encodes only the
        // Java exception path, which propagates and kills the parse.
        if !create_default_via_rule(sink, 0) {
            return false;
        }
    }
    // Every class gets `board.rules.getDefaultViaRule()` — the FIRST
    // rule in the table (or null).
    let default_id = sink.default_via_rule_id();
    for class in sink.net_classes_mut() {
        class.via_rule = default_id;
    }
    true
}

/// Java `Network.addViaRule` (`Network.java:398-421`): resolve the member
/// via infos by NAME; replace a same-named rule only when every member
/// resolved.
fn add_via_rule(name_list: &[String], sink: &mut dyn BoardSink) -> bool {
    let rule_name = &name_list[0];
    let existing_rule = sink.via_rule_no(rule_name);
    let mut members: Vec<i32> = Vec::new();
    let mut rule_ok = true;
    for name in &name_list[1..] {
        if let Some(via_info_no) = sink.via_info_no(name) {
            members.push(via_info_no);
        } else {
            // Java: "viaInfo not found" warn — log-only.
            rule_ok = false;
        }
    }
    if rule_ok {
        if let Some(id) = existing_rule {
            // Replace the already existing rule.
            sink.remove_via_rule(id);
        }
        sink.append_via_rule(rule_name.clone(), members);
    }
    rule_ok
}

/// Java `Network.insertNetClass` (`Network.java:436-543`): applies one
/// DSN class to the board. `default`/`kicad_default` (case-insensitive)
/// alias to board class 0; a null layer-rule entry kills the parse AFTER
/// earlier rules ran.
fn insert_net_class(class: &DsnNetClass, state: &ParseState, sink: &mut dyn BoardSink) -> bool {
    let Some(coordinate_transform) = state.coordinate_transform.as_ref() else {
        return false; // unreachable: the entry guard checked this
    };

    let board_class_idx = if is_kicad_default_net_class_name(&class.name) {
        0
    } else {
        sink.append_net_class(&class.name) as usize
    };

    if let Some(clearance_class_name) = &class.trace_clearance_class {
        let trace_clearance_class = sink
            .board_rules_mut()
            .clearance
            .get_no(clearance_class_name);
        if trace_clearance_class >= 0 {
            sink.net_classes_mut()[board_class_idx].trace_clearance_class = trace_clearance_class;
        } else {
            // Java: "clearance class not found" warn — log-only.
        }
    }
    if let Some(via_rule_name) = &class.via_rule {
        if let Some(id) = sink.via_rule_no(via_rule_name) {
            sink.net_classes_mut()[board_class_idx].via_rule = Some(id);
        } else {
            // Java: "via rule not found" warn — log-only.
        }
    }
    if class.max_trace_length > 0.0 {
        sink.net_classes_mut()[board_class_idx].max_trace_length =
            coordinate_transform.dsn_to_board_value(class.max_trace_length);
    }
    if class.min_trace_length > 0.0 {
        sink.net_classes_mut()[board_class_idx].min_trace_length =
            coordinate_transform.dsn_to_board_value(class.min_trace_length);
    }
    // Net membership (`:481-487`): `rules.nets.get(name)` returns EVERY
    // case-insensitive name match; a miss is an empty collection — a
    // SILENT no-op (no warn in Java).
    for net_name in &class.net_list {
        for net_no in sink.net_nos(net_name) {
            sink.nets_mut()[(net_no - 1) as usize].net_class = board_class_idx as i32;
        }
    }

    // Trace width and clearance rules.
    let mut clearance_rule_found = false;
    for rule in &class.rules {
        match rule {
            Rule::Width { value } => {
                // CLASS-rule division order: dsnToBoard(value / 2)
                // (`Network.java:485`); `setTraceHalfWidth(int)` fills
                // every layer.
                let trace_half_width =
                    java_round(coordinate_transform.dsn_to_board_value(value / 2.0)) as i32;
                for width in sink.net_classes_mut()[board_class_idx]
                    .trace_half_widths
                    .iter_mut()
                {
                    *width = trace_half_width;
                }
            }
            Rule::Clearance { value, pairs } => {
                add_clearance_rule(
                    sink,
                    board_class_idx,
                    *value,
                    pairs,
                    -1,
                    coordinate_transform,
                );
                clearance_rule_found = true;
            }
        }
    }

    // Layer-dependent rules. A null layer-rule entry NPEs HERE — after
    // everything above already mutated the board (ordering is
    // load-bearing; the lrnull battery pins the partial state).
    for layer_rule in &class.layer_rules {
        let Some(layer_rule) = layer_rule else {
            // Java: NPE on `currentLayerRule.layerNames` — parse death.
            return false;
        };
        for layer_name in &layer_rule.layer_names {
            // Java: `board.layerStructure.getNo` — the BOARD structure.
            let layer_index = sink
                .board_layer_structure()
                .map(|layers| layers.get_no(layer_name))
                .unwrap_or(-1);
            if layer_index < 0 {
                // Java: "layer not found" warn — log-only, continue.
                continue;
            }
            for rule in &layer_rule.rules {
                match rule {
                    Rule::Width { value } => {
                        let trace_half_width =
                            java_round(coordinate_transform.dsn_to_board_value(value / 2.0)) as i32;
                        let layer = layer_index as usize;
                        let net_class = &mut sink.net_classes_mut()[board_class_idx];
                        if layer < net_class.trace_half_widths.len() {
                            net_class.trace_half_widths[layer] = trace_half_width;
                        }
                    }
                    Rule::Clearance { value, pairs } => {
                        add_clearance_rule(
                            sink,
                            board_class_idx,
                            *value,
                            pairs,
                            layer_index,
                            coordinate_transform,
                        );
                        clearance_rule_found = true;
                    }
                }
            }
        }
    }

    sink.net_classes_mut()[board_class_idx].pull_tight = class.pull_tight;
    sink.net_classes_mut()[board_class_idx].shove_fixed = class.shove_fixed;

    let mut via_infos_created = false;
    // `boardNetClass != board.rules.getDefaultNetClass()` — reference
    // comparison; the KiCad alias IS the default object -> excluded.
    if clearance_rule_found && board_class_idx != 0 {
        create_default_via_infos(sink, board_class_idx, state.via_at_smd_allowed);
        via_infos_created = true;
    }

    if !class.use_via.is_empty() {
        create_via_rule(&class.use_via, board_class_idx, sink);
    } else if via_infos_created {
        // Java DISCARDS the boolean (`:541`); false = exception path =
        // parse death.
        if !create_default_via_rule(sink, board_class_idx) {
            return false;
        }
    }
    if !class.use_layer.is_empty() {
        create_active_trace_layers(&class.use_layer, state, sink, board_class_idx);
    }
    true
}

/// Java `Network.addClearanceRule` (`Network.java:730-789`): class-scope
/// clearance rule. Appends the class's own clearance-class row when new
/// (max-filled against the existing rows, item classes retargeted to it),
/// then applies the pair list (INERT unless a pair splits into exactly
/// two parts).
fn add_clearance_rule(
    sink: &mut dyn BoardSink,
    class_idx: usize,
    value: f64,
    pairs: &[String],
    layer_index: i32,
    coordinate_transform: &CoordinateTransform,
) {
    let current_clearance = java_round(coordinate_transform.dsn_to_board_value(value)) as i32;
    let class_name = sink.net_classes()[class_idx].name.clone();
    let mut class_no = sink.board_rules_mut().clearance.get_no(&class_name);
    if class_no < 0 {
        // Class not yet existing: append and max-fill the new row/column
        // against the existing values (`:744-756`).
        sink.board_rules_mut().clearance.append_class(&class_name);
        class_no = sink.board_rules_mut().clearance.get_no(&class_name);
        let class_count = sink.board_rules_mut().clearance.class_count();
        let layer_count = sink.board_rules_mut().clearance.values.len() as i32;
        for i in 1..class_count {
            for j in 0..layer_count {
                let current_value = sink
                    .board_rules_mut()
                    .clearance
                    .get_value(class_no, i, j)
                    .max(current_clearance);
                sink.board_rules_mut()
                    .clearance
                    .set_value(class_no, i, j, current_value);
                sink.board_rules_mut()
                    .clearance
                    .set_value(i, class_no, j, current_value);
            }
        }
        // `defaultItemClearanceClasses.setAll(classNo)`
        // (`rules/DefaultItemClearanceClasses.java:35-38`): the loop
        // starts at i = 1, so the never-read NONE slot 0 stays 0.
        sink.net_classes_mut()[class_idx].default_item_clearance_classes =
            [0, class_no, class_no, class_no, class_no, class_no];
    }
    sink.net_classes_mut()[class_idx].trace_clearance_class = class_no;

    if pairs.is_empty() {
        if layer_index < 0 {
            // The 3-arg setValue loops ALL layers (`ClearanceMatrix`).
            sink.board_rules_mut().clearance.set_value_all_layers(
                class_no,
                class_no,
                current_clearance,
            );
        } else {
            sink.board_rules_mut().clearance.set_value(
                class_no,
                class_no,
                layer_index,
                current_clearance,
            );
        }
        return;
    }
    if contains_wire_clearance_pair(pairs) {
        create_default_clearance_classes(sink, class_idx);
    }
    for current_string in pairs {
        let current_pair = java_split_underscore(current_string);
        if current_pair.len() != 2 {
            // `slow_kicad_default` -> 3 parts -> INERT (jar
            // /tmp/epic-t8-loopprobe.out).
            continue;
        }
        let first_class_no = get_clearance_class(sink, class_idx, current_pair[0]);
        let second_class_no = get_clearance_class(sink, class_idx, current_pair[1]);
        if layer_index < 0 {
            sink.board_rules_mut().clearance.set_value_all_layers(
                first_class_no,
                second_class_no,
                current_clearance,
            );
            sink.board_rules_mut().clearance.set_value_all_layers(
                second_class_no,
                first_class_no,
                current_clearance,
            );
        } else {
            sink.board_rules_mut().clearance.set_value(
                first_class_no,
                second_class_no,
                layer_index,
                current_clearance,
            );
            sink.board_rules_mut().clearance.set_value(
                second_class_no,
                first_class_no,
                layer_index,
                current_clearance,
            );
        }
    }
}

/// Java `Network.getClearanceClass` (`Network.java:795-844`): the
/// clearance class named `<class>[-<item>]` (`wire` maps to the class's
/// OWN name), created from the class's row when missing.
fn get_clearance_class(sink: &mut dyn BoardSink, class_idx: usize, item_class_name: &str) -> i32 {
    let net_class_name = sink.net_classes()[class_idx].name.clone();
    let new_class_name = if item_class_name == "wire" {
        net_class_name.clone()
    } else {
        format!("{}-{}", net_class_name, item_class_name)
    };
    let found_class_no = sink.board_rules_mut().clearance.get_no(&new_class_name);
    if found_class_no >= 0 {
        return found_class_no;
    }
    sink.board_rules_mut()
        .clearance
        .append_class(&new_class_name);
    let result = sink.board_rules_mut().clearance.get_no(&new_class_name);
    let net_class_no = sink.board_rules_mut().clearance.get_no(&net_class_name);
    if net_class_no < 0 || result < 0 {
        // Java: "clearance class not found" warn + the raw result.
        return result;
    }
    // Initialize the new row/column from the net class's row.
    let class_count = sink.board_rules_mut().clearance.class_count();
    let layer_count = sink.board_rules_mut().clearance.values.len() as i32;
    for i in 1..class_count {
        for j in 0..layer_count {
            let current_value = sink
                .board_rules_mut()
                .clearance
                .get_value(net_class_no, i, j);
            sink.board_rules_mut()
                .clearance
                .set_value(result, i, j, current_value);
            sink.board_rules_mut()
                .clearance
                .set_value(i, result, j, current_value);
        }
    }
    // Case-sensitive item-class slots; unsupported names ignored.
    let slot = match item_class_name {
        "via" => Some(ItemClassIr::Via),
        "pin" => Some(ItemClassIr::Pin),
        "smd" => Some(ItemClassIr::Smd),
        "area" => Some(ItemClassIr::Area),
        _ => None,
    };
    if let Some(slot) = slot {
        sink.net_classes_mut()[class_idx].default_item_clearance_classes[slot as usize] = result;
    }
    result
}

/// Java `Network.createDefaultClearanceClasses` (`Network.java:681-687`).
fn create_default_clearance_classes(sink: &mut dyn BoardSink, class_idx: usize) {
    get_clearance_class(sink, class_idx, "via");
    get_clearance_class(sink, class_idx, "smd");
    get_clearance_class(sink, class_idx, "pin");
    get_clearance_class(sink, class_idx, "area");
}

/// Java `Network.addMixedClearanceRule` (`Network.java:622-679`):
/// class-class clearance. Same pair-split rule as [`add_clearance_rule`],
/// but BOTH orderings per pair and NO wire-pair trigger. Missing class
/// rows are appended WITHOUT the max-fill/item-class retarget the
/// class-scope flavor does.
fn add_mixed_clearance_rule(
    sink: &mut dyn BoardSink,
    first_class_idx: usize,
    second_class_idx: usize,
    value: f64,
    pairs: &[String],
    layer_index: i32,
    coordinate_transform: &CoordinateTransform,
) {
    let current_clearance = java_round(coordinate_transform.dsn_to_board_value(value)) as i32;
    let first_class_name = sink.net_classes()[first_class_idx].name.clone();
    let mut first_class_no = sink.board_rules_mut().clearance.get_no(&first_class_name);
    if first_class_no < 0 {
        sink.board_rules_mut()
            .clearance
            .append_class(&first_class_name);
        first_class_no = sink.board_rules_mut().clearance.get_no(&first_class_name);
    }
    let second_class_name = sink.net_classes()[second_class_idx].name.clone();
    let mut second_class_no = sink.board_rules_mut().clearance.get_no(&second_class_name);
    if second_class_no < 0 {
        sink.board_rules_mut()
            .clearance
            .append_class(&second_class_name);
        second_class_no = sink.board_rules_mut().clearance.get_no(&second_class_name);
    }
    if pairs.is_empty() {
        if layer_index < 0 {
            sink.board_rules_mut().clearance.set_value_all_layers(
                first_class_no,
                second_class_no,
                current_clearance,
            );
            sink.board_rules_mut().clearance.set_value_all_layers(
                second_class_no,
                first_class_no,
                current_clearance,
            );
        } else {
            sink.board_rules_mut().clearance.set_value(
                first_class_no,
                second_class_no,
                layer_index,
                current_clearance,
            );
            sink.board_rules_mut().clearance.set_value(
                second_class_no,
                first_class_no,
                layer_index,
                current_clearance,
            );
        }
    } else {
        for current_string in pairs {
            let current_pair = java_split_underscore(current_string);
            if current_pair.len() != 2 {
                continue;
            }
            for i in 0..2 {
                let (current_first_class_no, current_second_class_no) = if i == 0 {
                    (
                        get_clearance_class(sink, first_class_idx, current_pair[0]),
                        get_clearance_class(sink, second_class_idx, current_pair[1]),
                    )
                } else {
                    (
                        get_clearance_class(sink, second_class_idx, current_pair[0]),
                        get_clearance_class(sink, first_class_idx, current_pair[1]),
                    )
                };
                if layer_index < 0 {
                    sink.board_rules_mut().clearance.set_value_all_layers(
                        current_first_class_no,
                        current_second_class_no,
                        current_clearance,
                    );
                    sink.board_rules_mut().clearance.set_value_all_layers(
                        current_second_class_no,
                        current_first_class_no,
                        current_clearance,
                    );
                } else {
                    sink.board_rules_mut().clearance.set_value(
                        current_first_class_no,
                        current_second_class_no,
                        layer_index,
                        current_clearance,
                    );
                    sink.board_rules_mut().clearance.set_value(
                        current_second_class_no,
                        current_first_class_no,
                        layer_index,
                        current_clearance,
                    );
                }
            }
        }
    }
}

/// Java `Network.insertClassPairs` (`Network.java:545-576`): the alias
/// iterator — for the FIRST class name that resolves, pair it with EVERY
/// subsequent name, then STOP scanning this scope.
fn insert_class_pairs(
    class_classes: &[DsnClassClass],
    state: &ParseState,
    sink: &mut dyn BoardSink,
) -> bool {
    for current_class_class in class_classes {
        for i in 0..current_class_class.class_names.len() {
            let first_name = &current_class_class.class_names[i];
            let Some(first_class_idx) = resolve_net_class_idx(first_name, sink) else {
                // Java: "first class not found" warn; the outer iterator
                // just advances.
                continue;
            };
            // `it2 = it1` alias: drain the rest, then the outer loop ends.
            for second_name in &current_class_class.class_names[i + 1..] {
                match resolve_net_class_idx(second_name, sink) {
                    None => {
                        // Java: "second class not found" warn.
                    }
                    Some(second_class_idx) => {
                        if !insert_class_pair_info(
                            current_class_class,
                            first_class_idx,
                            second_class_idx,
                            state,
                            sink,
                        ) {
                            return false;
                        }
                    }
                }
            }
            break;
        }
    }
    true
}

/// Java `Network.insertClassPairInfo` (`Network.java:578-620`).
fn insert_class_pair_info(
    class_class: &DsnClassClass,
    first_class_idx: usize,
    second_class_idx: usize,
    state: &ParseState,
    sink: &mut dyn BoardSink,
) -> bool {
    let Some(coordinate_transform) = state.coordinate_transform.as_ref() else {
        return false;
    };
    for rule in &class_class.rules {
        if let Rule::Clearance { value, pairs } = rule {
            add_mixed_clearance_rule(
                sink,
                first_class_idx,
                second_class_idx,
                *value,
                pairs,
                -1,
                coordinate_transform,
            );
        } else {
            // Java: "unexpected rule" warn — log-only.
        }
    }
    for layer_rule in &class_class.layer_rules {
        let Some(layer_rule) = layer_rule else {
            // Java: NPE on the null entry — parse death.
            return false;
        };
        for layer_name in &layer_rule.layer_names {
            // Java: `board.layerStructure.getNo` — the BOARD structure.
            let layer_index = sink
                .board_layer_structure()
                .map(|layers| layers.get_no(layer_name))
                .unwrap_or(-1);
            if layer_index < 0 {
                // Java: "layer not found" warn — log-only.
                continue;
            }
            for rule in &layer_rule.rules {
                if let Rule::Clearance { value, pairs } = rule {
                    add_mixed_clearance_rule(
                        sink,
                        first_class_idx,
                        second_class_idx,
                        *value,
                        pairs,
                        layer_index,
                        coordinate_transform,
                    );
                } else {
                    // Java: "unexpected layer rule type" warn — log-only.
                }
            }
        }
    }
    true
}

/// Java `KiCadNetClassNames.resolveNetClass` (`:31-38`) over the sink's
/// table: the KiCad default alias resolves to class 0, anything else is a
/// CASE-SENSITIVE name lookup.
fn resolve_net_class_idx(name: &str, sink: &mut dyn BoardSink) -> Option<usize> {
    if is_kicad_default_net_class_name(name) {
        return Some(0);
    }
    sink.net_classes()
        .iter()
        .position(|class| class.name == name)
}

/// Java `Network.createViaRule` (`Network.java:689-709`): a rule named
/// after the BOARD class holding every via info whose clearance class is
/// the class's VIA item AND whose padstack name equals a `use_via` entry
/// (exact match — `ViaP.1` matches nothing). Appended WITHOUT replacing.
/// (The Java `attachAllowed` parameter is dead in the body.)
fn create_via_rule(use_via: &[String], class_idx: usize, sink: &mut dyn BoardSink) {
    let class_name = sink.net_classes()[class_idx].name.clone();
    let default_via_cl_class =
        sink.net_classes()[class_idx].default_item_clearance_classes[ItemClassIr::Via as usize];
    let mut members: Vec<i32> = Vec::new();
    for current_via_name in use_via {
        for i in 0..sink.via_infos().len() {
            let via_info = &sink.via_infos()[i];
            if via_info.clearance_class == default_via_cl_class {
                let Some(padstack) = sink.padstack(via_info.padstack_no) else {
                    continue;
                };
                if padstack.name == *current_via_name {
                    members.push(i as i32);
                }
            }
        }
    }
    let id = sink.append_via_rule(class_name, members);
    sink.net_classes_mut()[class_idx].via_rule = Some(id);
}

/// Java `BoardRules.createDefaultViaRule` (`BoardRules.java:169-199`): a
/// rule named after the board class with the smallest-pad via per layer
/// range. Returns false only on the Java exception paths (an out-of-range
/// `getShape` AIOOBE or a null shape NPE — a padstack whose shapes are
/// all null makes `fromLayer()` == `shapes.len()`).
fn create_default_via_rule(sink: &mut dyn BoardSink, class_idx: usize) -> bool {
    if sink.via_infos().is_empty() {
        // Java: plain `return` — NOT a death.
        return true;
    }
    let class_name = sink.net_classes()[class_idx].name.clone();
    let default_via_cl_class =
        sink.net_classes()[class_idx].default_item_clearance_classes[ItemClassIr::Via as usize];
    // `members` are 0-based via-info indexes (Java ViaRule holds the
    // ViaInfo objects; the via-info table is append-only).
    let mut members: Vec<i32> = Vec::new();
    for i in 0..sink.via_infos().len() {
        let via_info = sink.via_infos()[i].clone();
        if via_info.clearance_class != default_via_cl_class {
            continue;
        }
        let Some(padstack) = sink.padstack(via_info.padstack_no) else {
            continue;
        };
        let current_from_layer = padstack_from_layer(&padstack.shapes);
        let current_to_layer = padstack_to_layer(&padstack.shapes);
        // `defaultRule.getLayerRange(from, to)`: the first member whose
        // padstack has the same layer range.
        let existing = members.iter().copied().position(|member| {
            let info = &sink.via_infos()[member as usize];
            let Some(member_padstack) = sink.padstack(info.padstack_no) else {
                return false;
            };
            padstack_from_layer(&member_padstack.shapes) == current_from_layer
                && padstack_to_layer(&member_padstack.shapes) == current_to_layer
        });
        match existing {
            Some(member_pos) => {
                let Some(new_shape_width) =
                    board_shape_max_width(&padstack.shapes, current_from_layer)
                else {
                    return false; // Java: getShape AIOOBE / null shape NPE
                };
                let existing_info = sink.via_infos()[members[member_pos] as usize].clone();
                let Some(existing_padstack) = sink.padstack(existing_info.padstack_no) else {
                    return false;
                };
                let Some(existing_shape_width) =
                    board_shape_max_width(&existing_padstack.shapes, current_from_layer)
                else {
                    return false; // Java: getShape AIOOBE / null shape NPE
                };
                if new_shape_width < existing_shape_width {
                    // The via with the smallest pad shape is preferred.
                    members.remove(member_pos);
                    members.push(i as i32);
                }
            }
            None => members.push(i as i32),
        }
    }
    let id = sink.append_via_rule(class_name, members);
    sink.net_classes_mut()[class_idx].via_rule = Some(id);
    true
}

/// Java `Network.createActiveTraceLayers` (`Network.java:711-728`): only
/// the listed layers stay active; inactive layers get half width 0. All
/// writes go through the bounds-guarded `rules.NetClass` setters
/// (`setActiveRoutingLayer`/`setTraceHalfWidth`, `NetClass.java:193-197`
/// and the hw setter), so a class table sized differently from the PARSER
/// structure (the Java parameter — not the board's) simply drops the
/// out-of-range writes.
fn create_active_trace_layers(
    use_layer: &[String],
    state: &ParseState,
    sink: &mut dyn BoardSink,
    class_idx: usize,
) {
    let Some(layer_structure) = state.layer_structure.as_ref() else {
        return; // unreachable: the entry guard checked this
    };
    let parser_layer_count = layer_structure.layers.len();
    for i in 0..parser_layer_count {
        let net_class = &mut sink.net_classes_mut()[class_idx];
        if i < net_class.active_routing_layers.len() {
            net_class.active_routing_layers[i] = false;
        }
    }
    for cur_layer_name in use_layer {
        let current_no = layer_structure.get_no(cur_layer_name);
        if current_no >= 0 {
            let net_class = &mut sink.net_classes_mut()[class_idx];
            let layer = current_no as usize;
            if layer < net_class.active_routing_layers.len() {
                net_class.active_routing_layers[layer] = true;
            }
        }
    }
    // Currently all inactive layers have trace width 0.
    for i in 0..parser_layer_count {
        // `isActiveRoutingLayer` returns FALSE out of range.
        let is_active = sink.net_classes()[class_idx]
            .active_routing_layers
            .get(i)
            .copied()
            .unwrap_or(false);
        if !is_active {
            let net_class = &mut sink.net_classes_mut()[class_idx];
            if i < net_class.trace_half_widths.len() {
                net_class.trace_half_widths[i] = 0;
            }
        }
    }
}

// ---- component / logical-part passes -------------------------------------

/// Java `Network.insertComponents` (`Network.java:832-838`).
fn insert_components(state: &ParseState, sink: &mut dyn BoardSink) {
    for placement in &state.placement_list {
        for location in &placement.locations {
            insert_component(location, &placement.lib_name, state, sink);
        }
    }
}

/// Java `Network.insertComponent` (`Network.java:948-1218`): adds the
/// board component, then its pins, package keepouts, and outlines. Every
/// failure inside is warn + return (the component stays whatever it was
/// when the failure hit) — no parse death.
fn insert_component(
    location: &ComponentLocation,
    lib_name: &str,
    state: &ParseState,
    sink: &mut dyn BoardSink,
) {
    let Some(coordinate_transform) = state.coordinate_transform.as_ref() else {
        return;
    };
    let Some(front_package_no) = sink.package_no(lib_name, true) else {
        // Java: "component package not found" warn + return (front AND
        // back are resolved up front; either miss aborts).
        return;
    };
    let Some(back_package_no) = sink.package_no(lib_name, false) else {
        return;
    };

    let component_location: Option<IntPoint> = location
        .coor
        .map(|coor| coordinate_transform.dsn_to_board_tuple(coor).round());
    // The Component ctor receives the RAW rotation (`:957-966`) and
    // NORMALIZES it internally with while loops (`Component.java:54-79`);
    // the obstacle/outline insertions below keep the RAW value.
    let rotation_in_degree = location.rotation;
    let normalized_rotation = normalize_rotation_java(rotation_in_degree);
    let fixed_state = if location.position_fixed {
        FixedStateIr::SystemFixed
    } else {
        FixedStateIr::Unfixed
    };
    let component_id = sink.insert_component(ComponentIr {
        name: location.name.clone(),
        package_front: front_package_no,
        package_back: back_package_no,
        location: component_location,
        rotation: normalized_rotation,
        is_front: location.is_front,
        fixed: fixed_state,
        part_number: location.part_number.clone(),
        logical_part: None,
    });

    let Some(component_location) = component_location else {
        return; // component is not yet placed
    };
    // `componentLocation.differenceBy(Point.ZERO)` == the location itself.
    let translation = component_location;

    // `newComponent.getPackage()` — the CURRENT-side package. Cloned to
    // detach from the sink for the mutation calls below.
    let current_package_no = if location.is_front {
        front_package_no
    } else {
        back_package_no
    };
    let Some(package) = sink.package(current_package_no).cloned() else {
        return; // unreachable: package_no just resolved it
    };

    // Pins (`Network.java:993-1050`).
    for (pin_index, package_pin) in package.pins.iter().enumerate() {
        let Some((from_layer, to_layer)) = sink
            .padstack(package_pin.padstack_no)
            .map(|padstack| padstack_from_to(&padstack.shapes))
        else {
            // Java: "pin padstack not found" warn + return (the component
            // stays PARTIAL — later pins are NOT inserted).
            return;
        };
        // `netlist.getNets(component, pin)`: the parser nets in TreeMap
        // key order (case-sensitive name, then subnet) containing the pin.
        let mut pin_net_numbers: Vec<i32> = Vec::new();
        for pin_net in state.netlist.get_nets(&location.name, &package_pin.name) {
            match sink.net_no_subnet(&pin_net.id.name, pin_net.id.subnet_number) {
                Some(net_no) => pin_net_numbers.push(net_no),
                None => {
                    // Java: "board net not found" warn — log-only, the
                    // net is skipped.
                }
            }
        }
        // The net class of the FIRST pin net, else the default class.
        let net_class_idx = pin_net_numbers
            .first()
            .map(|net_no| sink.nets()[(*net_no - 1) as usize].net_class)
            .unwrap_or(0);
        let mut clearance_class: i32 = -1;
        if let Some(pin_info) = location.pin_infos.get(&package_pin.name) {
            clearance_class = sink
                .board_rules_mut()
                .clearance
                .get_no(&pin_info.clearance_class);
        }
        if clearance_class < 0 {
            // `fromLayer() == toLayer()` — a padstack spanning exactly one
            // shape layer is SMD, anything else PIN.
            let item = if from_layer as i32 == to_layer {
                ItemClassIr::Smd
            } else {
                ItemClassIr::Pin
            };
            clearance_class = sink.net_classes()[net_class_idx as usize]
                .default_item_clearance_classes[item as usize];
        }
        sink.insert_pin(PinIr {
            component_id,
            pin_index: pin_index as i32,
            padstack_no: package_pin.padstack_no,
            nets: pin_net_numbers,
            clearance_class,
            fixed: fixed_state,
        });
    }

    // Package keepouts (`Network.java:1052-1171`): k = 0 keepout,
    // 1 via keepout, 2 place keepout.
    for k in 0..3u8 {
        let (keepouts, keepout_infos): (
            &[ImageKeepoutIr],
            &std::collections::BTreeMap<String, crate::state::ItemClearanceInfo>,
        ) = match k {
            0 => (&package.keepouts, &location.keepout_infos),
            1 => (&package.via_keepouts, &location.via_keepout_infos),
            _ => (&package.place_keepouts, &location.place_keepout_infos),
        };
        let kind = match k {
            0 => KeepoutKindIr::Keepout,
            1 => KeepoutKindIr::ViaKeepout,
            _ => KeepoutKindIr::PlaceKeepout,
        };
        for keepout in keepouts {
            let mut layer = keepout.layer_no;
            if layer >= sink.layer_count() {
                // Java: "keepout layer is to big" warn + continue.
                continue;
            }
            if layer >= 0 && !location.is_front {
                layer = sink.layer_count() - keepout.layer_no - 1;
            }
            // Clearance = the DEFAULT class's AREA item; a keepout_info
            // with a resolved class STRICTLY overrides (`> 0`).
            let mut clearance_class =
                sink.net_classes()[0].default_item_clearance_classes[ItemClassIr::Area as usize];
            if let Some(keepout_info) = keepout_infos.get(&keepout.name) {
                let current_clearance_class = sink
                    .board_rules_mut()
                    .clearance
                    .get_no(&keepout_info.clearance_class);
                if current_clearance_class > 0 {
                    clearance_class = current_clearance_class;
                }
            }
            let Some(area) = &keepout.area else {
                // Java passes the null area to insertObstacle, whose null
                // guard warns and skips (no id burned).
                continue;
            };
            let insert_one = |sink: &mut dyn BoardSink, layer: i32| {
                sink.insert_keepout(KeepoutIr {
                    kind,
                    layer_no: layer,
                    area: area.clone(),
                    clearance_class,
                    fixed: fixed_state,
                    component_id,
                    translation,
                    // RAW rotation (the Component ctor normalization does
                    // not reach the obstacle arguments).
                    rotation: rotation_in_degree,
                    side_changed: !location.is_front,
                    name: if keepout.name.is_empty() {
                        None
                    } else {
                        Some(keepout.name.clone())
                    },
                });
            };
            if layer >= 0 {
                insert_one(sink, layer);
            } else {
                // Insert on all BOARD signal layers (indexes collected
                // first — the structure read and the insertion cannot
                // borrow the sink at once).
                let signal_layers: Vec<i32> = match sink.board_layer_structure() {
                    Some(board_layers) => board_layers
                        .layers
                        .iter()
                        .enumerate()
                        .filter(|(_, board_layer)| board_layer.is_signal)
                        .map(|(j, _)| j as i32)
                        .collect(),
                    None => continue,
                };
                for signal_layer in signal_layers {
                    insert_one(sink, signal_layer);
                }
            }
        }
    }

    // Outline as component keepout (`Network.java:1172-1217`).
    let outlines = &package.outline;
    let mut courtyard_idx: i32 = -1;
    if outlines.len() > 1 {
        let mut max_area = -1.0f64;
        for (i, outline) in outlines.iter().enumerate() {
            let Some(shape) = &outline.shape else {
                continue; // Java: `outline[i] != null` guard
            };
            if !shape_is_bounded(shape) {
                // Java `boundingBox() != null` guard — an empty area has
                // no bounding box (degenerate polygons never appear in
                // exporter output).
                continue;
            }
            let area = shape.bounding_box().area();
            if area > max_area {
                max_area = area;
                courtyard_idx = i as i32;
            }
        }
    }
    for (i, outline) in outlines.iter().enumerate() {
        let mut is_courtyard = i as i32 == courtyard_idx;
        if outline.width == 0.0 {
            is_courtyard = true;
        }
        let mut is_fabrication = false;
        if !is_courtyard && outline.width <= 110.0 {
            is_fabrication = true;
        }
        let is_closed = outline.is_closed;
        let area = outline.shape.as_ref().and_then(|shape| {
            // Java `BasicBoard.insertComponentOutline` skips an UNBOUNDED
            // area with a warn (no id burned); the sink only guards None,
            // so the reader folds unbounded shapes to None.
            if shape_is_bounded(shape) {
                Some(AreaIr::simple(shape.clone()))
            } else {
                None
            }
        });
        sink.insert_component_outline(ComponentOutlineIr {
            component_id,
            layer_no: 0, // the ComponentOutline ctor's constant 0 layer
            area,
            nets: Vec::new(),
            clearance_class: 0,
            fixed: fixed_state,
            is_front: location.is_front,
            translation,
            rotation: rotation_in_degree,
            is_courtyard,
            is_fabrication,
            is_closed,
        });
    }
}

/// Java `Network.insertLogicalParts` (`Network.java:846-912`). Returns
/// false when a logical part or package pin is missing — the CALLER
/// DISCARDS the result, so only the warnings matter.
fn insert_logical_parts(state: &ParseState, sink: &mut dyn BoardSink) -> bool {
    for next_part in &state.logical_parts {
        let Some(lib_package_no) = search_lib_package(&next_part.name, state, sink) else {
            return false;
        };
        let mut board_part_pins: Vec<LogicalPartPinIr> = Vec::new();
        for part_pin in &next_part.part_pins {
            let Some(pin_index) = package_pin_index(sink, lib_package_no, &part_pin.pin_name)
            else {
                // Java: "package pin not found" warn + false.
                return false;
            };
            board_part_pins.push(LogicalPartPinIr {
                pin_index,
                pin_name: part_pin.pin_name.clone(),
                gate_name: part_pin.gate_name.clone(),
                gate_swap_code: part_pin.gate_swap_code,
                gate_pin_name: part_pin.gate_pin_name.clone(),
                gate_pin_swap_code: part_pin.gate_pin_swap_code,
            });
        }
        sink.append_logical_part(LogicalPartIr {
            name: next_part.name.clone(),
            pins: board_part_pins,
        });
    }

    for next_mapping in &state.logical_part_mappings {
        let current_logical_part = sink.logical_part_name(&next_mapping.name);
        if current_logical_part.is_none() {
            // Java: "logical part not found" warn — and the flow CONTINUES
            // (components still get setLogicalPart(null)).
        }
        for current_cmp_name in &next_mapping.components {
            // Components.get: case-SENSITIVE; a miss is the sink no-op
            // (Java warns "board component not found" — log-only).
            sink.set_component_logical_part(current_cmp_name, current_logical_part.clone());
        }
    }
    true
}

/// Java `Network.searchLibPackage` (`Network.java:918-945`): the package
/// of the FIRST component (sorted-first — the mapping set is a TreeSet)
/// of the mapping with the part's name, via the BOARD component instance.
fn search_lib_package(
    part_name: &str,
    state: &ParseState,
    sink: &mut dyn BoardSink,
) -> Option<i32> {
    for current_mapping in &state.logical_part_mappings {
        if current_mapping.name != part_name {
            continue;
        }
        if current_mapping.components.is_empty() {
            // Java: "component list empty" warn + null.
            return None;
        }
        let component_name = current_mapping.components.first()?;
        let Some(component_package_no) = sink.component_package_no(component_name) else {
            // Java: "component not found" warn + null.
            return None;
        };
        return Some(component_package_no);
    }
    // Java: "library package not found" warn + null.
    None
}

/// Java `Package.getPinIndex` — the case-SENSITIVE position of the pin
/// name in the package's pin list.
fn package_pin_index(sink: &mut dyn BoardSink, package_no: i32, pin_name: &str) -> Option<i32> {
    sink.package_pins(package_no)
        .iter()
        .position(|pin| pin.name == pin_name)
        .map(|index| index as i32)
}

// ---- helpers -------------------------------------------------------------

/// Java `KiCadNetClassNames.isKiCadDefaultNetClassName` — non-empty and
/// case-insensitively "default" or "kicad_default".
fn is_kicad_default_net_class_name(name: &str) -> bool {
    !name.is_empty() && (eq_ignore_case(name, "default") || eq_ignore_case(name, "kicad_default"))
}

/// Java `String.split("_")`: split at EVERY underscore, drop TRAILING
/// empty parts (interior empties survive: `a__b` -> ["a", "", "b"]).
fn java_split_underscore(value: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = value.split('_').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

/// Java `Padstack.fromLayer()` (`Padstack.java:137-144`): the first layer
/// with a shape; `shapes.len()` when ALL are null.
pub(crate) fn padstack_from_layer(shapes: &[Option<BoardShape>]) -> usize {
    shapes
        .iter()
        .position(|shape| shape.is_some())
        .unwrap_or(shapes.len())
}

/// Java `Padstack.toLayer()` (`Padstack.java:146-153`): the last layer
/// with a shape; -1 when ALL are null.
pub(crate) fn padstack_to_layer(shapes: &[Option<BoardShape>]) -> i32 {
    shapes
        .iter()
        .rposition(|shape| shape.is_some())
        .map_or(-1, |index| index as i32)
}

/// [`padstack_from_layer`] + [`padstack_to_layer`] as one.
fn padstack_from_to(shapes: &[Option<BoardShape>]) -> (usize, i32) {
    (padstack_from_layer(shapes), padstack_to_layer(shapes))
}

/// Java `ConvexShape.maxWidth()` through [`BoardShape`]: implemented on
/// the tile family and circle only (Java: IntBox/IntOctagon/Simplex/
/// Circle; a PolygonShape padstack would have died with a
/// ClassCastException at padstack creation, so `None` = the Java death
/// path).
fn board_shape_max_width(shapes: &[Option<BoardShape>], layer: usize) -> Option<f64> {
    let shape = shapes.get(layer)?;
    match shape.as_ref()? {
        BoardShape::Tile(tile) => Some(tile.max_width()),
        BoardShape::Circle(circle) => Some(circle.max_width()),
        BoardShape::PolygonShape(_) => None,
    }
}

/// Java `Area.isBounded()` over the concrete shapes: tiles are always
/// bounded, polygons carry the flag, a circle is bounded when its radius
/// is positive.
fn shape_is_bounded(shape: &BoardShape) -> bool {
    match shape {
        BoardShape::Tile(_) => true,
        BoardShape::PolygonShape(polygon) => polygon.is_bounded(),
        BoardShape::Circle(circle) => circle.radius > 0,
    }
}

/// Java `Component` ctor rotation normalization (`Component.java:54-79`):
/// while loops, NOT a modulo — `-45.5` -> `314.5`, `720.5` -> `0.5`.
fn normalize_rotation_java(rotation: f64) -> f64 {
    let mut result = rotation;
    while result >= 360.0 {
        result -= 360.0;
    }
    while result < 0.0 {
        result += 360.0;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ses_board::{ItemIr, SesBoard};

    /// Runs the scope readers over fixture bodies in DsnFile order
    /// (structure -> placement -> library -> part_library -> network) and
    /// returns the network outcome plus the final state and board. Each
    /// body is fed to its reader through its OWN scanner positioned after
    /// the scope opener (house style; the readers are scope-local and the
    /// dispatcher consumes the OPEN bracket + keyword before each call).
    fn run_board(
        structure_body: &str,
        placement_body: Option<&str>,
        library_body: Option<&str>,
        part_library_body: Option<&str>,
        network_body: &str,
    ) -> (bool, ParseState, SesBoard) {
        let mut state = ParseState::default();
        state.unit = crate::state::Unit::Um;
        state.resolution = 10;
        let mut board = SesBoard::new();

        let text = format!("(structure {structure_body})");
        let mut scanner = Scanner::new(text.as_bytes());
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Structure));
        assert!(crate::scope::structure::read_scope(
            &mut scanner,
            &mut state,
            &mut board
        ));

        if let Some(body) = placement_body {
            let text = format!("(placement {body})");
            let mut scanner = Scanner::new(text.as_bytes());
            assert_eq!(scanner.next_token(), Token::Open);
            assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Placement));
            assert!(crate::scope::placement::read_placement_scope(
                &mut scanner,
                &mut state,
                &mut board
            ));
        }

        if let Some(body) = library_body {
            let text = format!("(library {body})");
            let mut scanner = Scanner::new(text.as_bytes());
            assert_eq!(scanner.next_token(), Token::Open);
            assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Library));
            assert!(crate::scope::library::read_library_scope(
                &mut scanner,
                &mut state,
                &mut board
            ));
        }

        if let Some(body) = part_library_body {
            let text = format!("(part_library {body})");
            let mut scanner = Scanner::new(text.as_bytes());
            assert_eq!(scanner.next_token(), Token::Open);
            assert_eq!(scanner.next_token(), Token::Keyword(Keyword::PartLibrary));
            assert!(crate::scope::library::read_part_library_scope(
                &mut scanner,
                &mut state.logical_part_mappings,
                &mut state.logical_parts
            ));
        }

        // The trailing `)` emulates the PCB scope's closing bracket, which
        // always follows the network scope in a real file. Load-bearing for
        // the bare-class battery: the class reader's mis-consumption eats
        // the NETWORK close, and the jar absorbs this by breaking the
        // network dispatch on the PCB close (tail runs) — the outer
        // `ScopeKeyword.readScope` then ends cleanly on EOF (`if
        // (nextToken == null) return true`). Without the extra bracket the
        // over-consumption lands on EOF here and would diverge from the
        // jar's file-level Success.
        let text = format!("(network {network_body}))");
        let mut scanner = Scanner::new(text.as_bytes());
        assert_eq!(scanner.next_token(), Token::Open);
        assert_eq!(scanner.next_token(), Token::Keyword(Keyword::Network));
        let ok = read_scope(&mut scanner, &mut state, &mut board);
        (ok, state, board)
    }

    const TWO_LAYERS: &str = "(layer F.Cu (type signal)) (layer B.Cu (type signal)) \
(boundary (path pcb 0  0 0  10000 0  10000 8000  0 8000  0 0))";

    /// Jar `/tmp/epic-t8-net.dsn` -> `/tmp/epic-t8-probe.out` FILE 1: the
    /// full net battery. Pins the net/subnet append and TreeMap pin order,
    /// the net-scope width `getNewNetClass` chain (class1/2/3 for
    /// VDD/FAST/SEC, the second VDD subnet REUSING class1 via
    /// `NetClasses.find`), the class tail (kicad_default alias -> class 0,
    /// its net_list membership overriding the earlier width classes for
    /// BOTH VDD subnets and matching `gnd` case-insensitively), the
    /// via-info read-time fallback appends vs the tail list REPLACE, the
    /// three-stage via-rule ordering (tail replace -> createViaRule
    /// "default" [] -> createDefaultViaRule "slow"), the wire-pair item
    /// classes, the class_class pair, and the circuit length/use_layer
    /// effects (B.Cu half width 0, the length x10 transform).
    #[test]
    fn t8_net_battery_jar_probe() {
        let (ok, state, board) = run_board(
            &format!("{TWO_LAYERS} (rule (clearance 200))"),
            Some(
                "(component PAD (place R1 2000 2000 front 0) (place R2 4000 2000 front 0)) \
                 (component SOIC (place U1 2000 5000 front 0)) \
                 (component OTHER (place U9))",
            ),
            Some(
                "(padstack ViaP (shape (circle signal 600))) \
                 (padstack ViaQ (shape (circle signal 500) (attach on))) \
                 (padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (padstack S1 (shape (rect F.Cu -400 -200 400 200)) (attach off)) \
                 (image PAD (pin P1 1 0 0) (pin P1 2 0 1000)) \
                 (image SOIC (pin S1 1 0 0) (pin S1 2 1000 0)) \
                 (image OTHER (pin P1 1 0 0))",
            ),
            None,
            "(net GND (pins R1-1 R2-1)) \
             (net gnd (pins U1-2)) \
             (net VDD (pins R1-2 R2-2) (rule (width 600))) \
             (net VDD 2 (fromto R1-2 R2-2)) \
             (net FAST (pins R2-2 U1-1) (rule (width 400) (clearance 500 (type fast_slow)))) \
             (net SEC (pins U1-2) (rule (width 800))) \
             (net SEC (pins R1-1)) \
             (via VIA1 ViaP slow) \
             (via VIA2 ViaQ default attach) \
             (via_rule slow VIA1 VIA2) \
             (via_rule slow VIA2) \
             (class kicad_default GND VDD \
               (circuit (length 20000 5000) (use_via ViaP.1) (use_layer F.Cu)) \
               (rule (width 300) (clearance 800 (type wire_via)))) \
             (class slow SEC \
               (rule (width 700) (clearance 900 (type slow_kicad_default))) \
               (layer_rule F.Cu (rule (width 300)))) \
             (class_class (classes kicad_default slow) \
               (rule (clearance 1200)))",
        );
        assert!(ok, "jar RESULT Success");
        assert!(state.warnings.is_empty(), "jar WARNINGS 0");

        // NET rows 1-6: name, subnet, net_class.
        let nets: Vec<(&str, i32, i32)> = board
            .nets
            .iter()
            .map(|net| (net.name.as_str(), net.subnet_number, net.net_class))
            .collect();
        assert_eq!(
            nets,
            [
                ("GND", 1, 0),
                ("gnd", 1, 0),
                ("VDD", 1, 0),
                ("VDD", 2, 0),
                ("FAST", 1, 2),
                ("SEC", 1, 4),
            ]
        );

        // Net classes 0-4. The jar probe prints via/pin/smd/area item
        // classes (slots 2-5); the never-read NONE slot stays 0
        // (`DefaultItemClearanceClasses.setAll` loops from i = 1) and
        // TRACE holds the create_board/default value 1 here (no setAll
        // fired for the existing "default" class) — slot checks on the
        // setAll path live in the wire-pair test below.
        assert_eq!(board.net_classes.len(), 5);
        let item_slots = |idx: usize| {
            let items = board.net_classes[idx].default_item_clearance_classes;
            (items[2], items[3], items[4], items[5])
        };
        assert_eq!(board.net_classes[0].name, "default");
        assert_eq!(board.net_classes[0].trace_clearance_class, 1);
        assert_eq!(board.net_classes[0].trace_half_widths, [1500, 0]);
        assert_eq!(board.net_classes[0].active_routing_layers, [true, false]);
        assert_eq!(item_slots(0), (2, 4, 3, 5), "wire pair: via smd pin area");
        assert_eq!(board.net_classes[0].min_trace_length, 50000.0);
        assert_eq!(board.net_classes[0].max_trace_length, 200000.0);
        assert!(board.net_classes[0].pull_tight && !board.net_classes[0].shove_fixed);
        assert_eq!(board.net_classes[1].name, "class1");
        assert_eq!(board.net_classes[1].trace_half_widths, [3000, 3000]);
        assert_eq!(item_slots(1), (1, 1, 1, 1), "getNewNetClass fresh DICCs");
        assert_eq!(board.net_classes[2].name, "class2");
        assert_eq!(board.net_classes[2].trace_half_widths, [2000, 2000]);
        assert_eq!(board.net_classes[3].name, "class3");
        assert_eq!(board.net_classes[3].trace_half_widths, [4000, 4000]);
        assert_eq!(board.net_classes[4].name, "slow");
        assert_eq!(board.net_classes[4].trace_clearance_class, 6);
        assert_eq!(board.net_classes[4].trace_half_widths, [1500, 3500]);
        assert_eq!(item_slots(4), (6, 6, 6, 6), "setAll(slow row) on append");

        // Via infos: the read-time clearance-class misses fall back to 1
        // ("slow" does not exist yet); the class-4 defaults carry cls 6.
        let infos: Vec<(&str, i32, i32, bool)> = board
            .via_infos
            .iter()
            .map(|info| {
                (
                    info.name.as_str(),
                    info.padstack_no,
                    info.clearance_class,
                    info.attach_smd_allowed,
                )
            })
            .collect();
        assert_eq!(
            infos,
            [
                ("VIA1", 1, 1, false),
                ("VIA2", 2, 1, true),
                ("ViaP-slow", 1, 6, false)
            ]
        );

        // Via rules in table order: tail addViaRule+replace -> the
        // kicad_default createViaRule (ViaP.1 matches no padstack name)
        // -> the class-4 createDefaultViaRule. DEFAULT is the first.
        let rules: Vec<(&str, &[i32])> = board
            .via_rules
            .iter()
            .map(|rule| (rule.name.as_str(), rule.via_infos.as_slice()))
            .collect();
        assert_eq!(
            rules,
            [("slow", &[1][..]), ("default", &[]), ("slow", &[2])]
        );
        assert_eq!(board.default_via_rule_id(), Some(board.via_rules[0].id));
        assert_eq!(board.net_classes[0].via_rule, Some(board.via_rules[1].id));
        assert_eq!(board.net_classes[1].via_rule, Some(board.via_rules[0].id));
        assert_eq!(board.net_classes[2].via_rule, Some(board.via_rules[0].id));
        assert_eq!(board.net_classes[3].via_rule, Some(board.via_rules[0].id));
        assert_eq!(board.net_classes[4].via_rule, Some(board.via_rules[2].id));

        // The tail REPLACE ([ViaP.1] cleaned) wiped the read-time appends.
        assert_eq!(board.via_padstacks, [1]);

        // Clearance classes and the layer-0 matrix rows 1/2/6.
        assert_eq!(
            board.rules.clearance.names,
            [
                "null",
                "default",
                "default-via",
                "default-smd",
                "default-pin",
                "default-area",
                "slow"
            ]
        );
        let m = &board.rules.clearance.values[0];
        assert_eq!(m[1], [0, 2000, 8000, 2000, 2000, 2000, 12000]);
        assert_eq!(m[2], [0, 8000, 2000, 2000, 2000, 2000, 9000]);
        assert_eq!(m[6], [0, 12000, 9000, 9000, 9000, 9000, 9000]);
        assert_eq!(board.rules.clearance.values[1][1][2], 8000, "all-layers");

        // Components in placement order; U9 unplaced.
        let names: Vec<&str> = board.components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["R1", "R2", "U1", "U9"]);
        assert_eq!(
            board.components[0].location,
            Some(IntPoint { x: 20000, y: 20000 })
        );
        assert_eq!(
            board.components[2].location,
            Some(IntPoint { x: 20000, y: 50000 })
        );
        assert_eq!(board.components[3].location, None);
        assert!(board.components.iter().all(|c| c.logical_part.is_none()));

        // Pins in insertion order (ids 2..7 after the board outline).
        let pins: Vec<(i32, &PinIr)> = board
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::Pin { id, pin } => Some((*id, pin)),
                _ => None,
            })
            .collect();
        assert_eq!(pins.len(), 6);
        let row = |i: usize| (pins[i].0, pins[i].1.component_id, pins[i].1.pin_index);
        assert_eq!(row(0), (2, 1, 0));
        assert_eq!(row(5), (7, 3, 1));
        // R1-1: nets GND(1)+SEC(6) in TreeMap order, default-class pin 4.
        assert_eq!(pins[0].1.nets, [1, 6]);
        assert_eq!(pins[0].1.padstack_no, 3);
        assert_eq!(pins[0].1.clearance_class, 4);
        // R2-2: FAST first -> class2 pin slot 1.
        assert_eq!(pins[3].1.nets, [5, 3, 4]);
        assert_eq!(pins[3].1.clearance_class, 1);
        // U1-1 is SMD on S1 -> class2 smd slot 1; U1-2 class0 smd 3.
        assert_eq!(pins[4].1.nets, [5]);
        assert_eq!(pins[4].1.clearance_class, 1);
        assert_eq!(pins[5].1.nets, [1]);
        assert_eq!(pins[5].1.clearance_class, 3);

        assert!(board.logical_parts.is_empty());
    }

    /// Jar `/tmp/epic-t8-structvia.dsn` -> probe FILE 2: the
    /// STRUCTURE-SEEDED tail. The `(via ViaP ViaQ (spare ViaSpare))` name
    /// list seeds `viaPadstackNames`, the class use_via appends `ViaP.1`,
    /// and the tail REPLACE resolves to `[ViaP ViaQ ViaSpare ViaP]` (no
    /// dedup). `ViaInfos.add` then drops the duplicate name, so 4
    /// padstacks yield 3 default infos; createDefaultViaRule picks the
    /// smallest same-range pad (ViaSpare) and the kicad_default
    /// createViaRule appends an empty second `default` rule.
    #[test]
    fn t8_structvia_seeded_tail_jar_probe() {
        let (ok, _state, board) = run_board(
            &format!("{TWO_LAYERS} (via ViaP ViaQ (spare ViaSpare))"),
            Some("(component PAD (place R1 2000 2000 front 0))"),
            Some(
                "(padstack ViaP (shape (circle signal 600))) \
                 (padstack ViaQ (shape (circle signal 500))) \
                 (padstack ViaSpare (shape (circle signal 400))) \
                 (padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (image PAD (pin P1 1 0 0))",
            ),
            None,
            "(net GND (pins R1-1)) \
             (class kicad_default GND (circuit (use_via ViaP.1)))",
        );
        assert!(ok);
        assert_eq!(
            board.via_padstacks,
            [1, 2, 3, 1],
            "structure seed + use_via append, replace without dedup"
        );
        let names: Vec<&str> = board.via_infos.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            ["ViaP", "ViaQ", "ViaSpare"],
            "duplicate info dropped"
        );
        assert!(
            board
                .via_infos
                .iter()
                .all(|i| i.clearance_class == 1 && !i.attach_smd_allowed)
        );
        let rules: Vec<(&str, &[i32])> = board
            .via_rules
            .iter()
            .map(|rule| (rule.name.as_str(), rule.via_infos.as_slice()))
            .collect();
        assert_eq!(rules, [("default", &[2][..]), ("default", &[])]);
        assert_eq!(board.default_via_rule_id(), Some(board.via_rules[0].id));
        // createViaRule OVERWRITES the class's rule set by
        // createDefaultViaRule (jar: nc0.via = rules[1]).
        assert_eq!(board.net_classes[0].via_rule, Some(board.via_rules[1].id));
        assert_eq!(board.net_classes.len(), 1);
        assert_eq!(board.net_classes[0].trace_half_widths, [1500, 1500]);
        assert_eq!(board.nets.len(), 1);
        assert_eq!(board.nets[0].net_class, 0);
    }

    /// Jar `/tmp/epic-t8-pins.dsn` -> probe FILE 3: the component battery.
    /// Pins the Component-ctor rotation normalization (-45.5 -> 314.5,
    /// 720.5 -> 0.5), the back-side keepout layer flip
    /// (`layerCount - orig - 1` over 3 layers), the dense id sequence
    /// (outline 1; per component 2 pins + OA + VOA + COA + 4 COUTs), the
    /// outline classification (max-bbox-area courtyard, width==0 forced
    /// courtyard, width<=110 fabrication), the unresolvable
    /// `clearance_class power` pin-info fallback, and the dense
    /// unplaced-last component list.
    #[test]
    fn t8_pins_battery_jar_probe() {
        let (ok, _state, board) = run_board(
            "(layer F.Cu (type signal)) (layer IN1 (type signal)) (layer B.Cu (type signal)) \
             (boundary (path pcb 0  0 0  10000 0  10000 8000  0 8000  0 0))",
            Some(
                "(component MIX \
                 (place K1 2000 2000 front -45.5 (pin 1 (clearance_class power))) \
                 (place K2 6000 2000 front 720.5) \
                 (place K3 4000 5000 back 0))",
            ),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (padstack S1 (shape (rect F.Cu -400 -200 400 200)) (attach off)) \
                 (image MIX \
                 (pin P1 1 0 0) \
                 (pin S1 2 2000 0) \
                 (keepout (rect IN1 -500 -500 500 500)) \
                 (via_keepout (circle B.Cu 300 2500 0)) \
                 (place_keepout (rect F.Cu -500 2000 500 3000)) \
                 (outline (path signal 111 0 0 2000 0)) \
                 (outline (path signal 101 0 0 2000 4000)) \
                 (outline (rect signal 0 0 1000 1000)) \
                 (outline (path signal 50 3000 0 3400 0)))",
            ),
            None,
            "(net GND (pins K1-2 K3-1)) (class kicad_default GND)",
        );
        assert!(ok, "jar RESULT Success");

        // Rotations: the Component ctor normalizes, the obstacles keep the
        // RAW value (checked via the K1 outline below).
        assert_eq!(board.components[0].rotation, 314.5);
        assert_eq!(board.components[1].rotation, 0.5);
        assert_eq!(board.components[2].rotation, 0.0);
        assert!(!board.components[2].is_front, "K3 on the back");
        assert_eq!(
            board.components[2].location,
            Some(IntPoint { x: 40000, y: 50000 })
        );

        // The dense id walk: outline 1, then per component 2 pins + OA +
        // VOA + COA + 4 COUTs.
        let mut kinds: Vec<(i32, &str)> = Vec::new();
        let mut cout_flags: Vec<(i32, bool, bool, bool)> = Vec::new();
        let mut keepout_layers: Vec<(i32, i32, bool)> = Vec::new();
        for item in &board.items {
            match item {
                ItemIr::BoardOutline { id, .. } => kinds.push((*id, "outline")),
                ItemIr::Pin { id, .. } => kinds.push((*id, "pin")),
                ItemIr::Keepout { id, keepout } => {
                    kinds.push((*id, "keepout"));
                    keepout_layers.push((*id, keepout.layer_no, keepout.side_changed));
                }
                ItemIr::ComponentOutline { id, outline } => {
                    kinds.push((*id, "cout"));
                    cout_flags.push((
                        *id,
                        outline.is_courtyard,
                        outline.is_fabrication,
                        outline.is_closed,
                    ));
                }
                _ => kinds.push((item.id(), "other")),
            }
        }
        assert_eq!(kinds.len(), 28);
        assert_eq!(kinds[0], (1, "outline"));
        // K1 (front): pins 2,3; OA 4; VOA 5; COA 6; COUTs 7-10.
        assert_eq!(
            &kinds[1..7],
            &[
                (2, "pin"),
                (3, "pin"),
                (4, "keepout"),
                (5, "keepout"),
                (6, "keepout"),
                (7, "cout")
            ]
        );
        // K2 continues at 11; K3 at 20 with pins 20,21.
        assert_eq!(kinds[10], (11, "pin"));
        assert_eq!(kinds[19], (20, "pin"));
        assert_eq!(kinds[20], (21, "pin"));
        assert_eq!(kinds[21], (22, "keepout"));
        assert_eq!(kinds[27], (28, "cout"));

        // K3 back-side flip over 3 layers: keepout 1 -> 1, via 2 -> 0,
        // place 0 -> 2; side_changed set.
        assert_eq!(
            &keepout_layers[..],
            &[
                (4, 1, false),
                (5, 2, false),
                (6, 0, false),
                (13, 1, false),
                (14, 2, false),
                (15, 0, false),
                (22, 1, true),
                (23, 0, true),
                (24, 2, true),
            ]
        );

        // Outline classification per component: o0 plain (width 111),
        // o1 the max-area courtyard, o2 width==0 forced courtyard (and
        // closed: rect), o3 width 50 fabrication.
        assert_eq!(
            &cout_flags[..4],
            &[
                (7, false, false, false),
                (8, true, false, false),
                (9, true, false, true),
                (10, false, true, false)
            ]
        );
        // Jar probe FILE 3: K2 COUTs ids 16-19, K3 ids 25-28, the same
        // flag pattern as K1 (`COUT id=19 comp=2 courtyard=false
        // fabrication=true closed=false` / `COUT id=28 comp=3 ...`).
        assert_eq!(
            &cout_flags[4..8],
            &[
                (16, false, false, false),
                (17, true, false, false),
                (18, true, false, true),
                (19, false, true, false)
            ]
        );
        assert_eq!(cout_flags[10], (27, true, false, true));
        assert_eq!(cout_flags[11], (28, false, true, false));

        // Pin nets/classes: K1-2 (S1) in GND -> class0 pin 1; K1-1's
        // `power` pin info does not resolve -> the class-0 PIN item (1).
        let pins: Vec<&PinIr> = board
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::Pin { pin, .. } => Some(pin),
                _ => None,
            })
            .collect();
        assert_eq!(pins.len(), 6);
        assert_eq!(pins[1].nets, [1]);
        assert_eq!(pins[1].padstack_no, 2, "S1");
        assert_eq!(pins[0].nets, Vec::<i32>::new(), "power class miss");
        assert_eq!(pins[0].clearance_class, 1);
        assert_eq!(pins[4].nets, [1], "K3-1");
    }

    /// Jar `/tmp/epic-t8-logical.dsn` -> probe FILE 4: the logical part
    /// with the missing package pin '99'. `insert_logical_parts` bails
    /// with false, the CALLER DISCARDS it — RESULT Success, zero parts
    /// inserted (LPB dies first, LPA never runs), no mapping applied.
    #[test]
    fn t8_logical_missing_pin_warn_only() {
        let (ok, _state, board) = run_board(
            TWO_LAYERS,
            Some("(component PAD (place R1 2000 2000 front 0) (place R2 4000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (image PAD (pin P1 1 0 0) (pin P1 2 0 1000))",
            ),
            Some(
                "(logical_part LPB (pin 99 1 G1 0 1 0)) \
                 (logical_part LPA (pin 1 1 G1 0 1 0) (pin 2 2 G1 0 2 0)) \
                 (logical_part_mapping LPA (comp R1)) \
                 (logical_part_mapping LPB (comp R1 R2))",
            ),
            "(net GND (pins R1-1 R2-1)) (class kicad_default GND)",
        );
        assert!(ok, "jar RESULT Success despite the discarded false");
        assert!(board.logical_parts.is_empty(), "jar LOGICALPART_COUNT 0");
        assert!(
            board.components.iter().all(|c| c.logical_part.is_none()),
            "jar lp=null"
        );
    }

    /// Jar `/tmp/epic-t8-logical2.dsn` -> probe FILE 5: both parts insert,
    /// and the mappings apply in file order so the LATER LPB mapping wins
    /// for R1 (setLogicalPart overwrites) — lp=LPB for both components.
    #[test]
    fn t8_logical2_mappings() {
        let (ok, _state, board) = run_board(
            TWO_LAYERS,
            Some("(component PAD (place R1 2000 2000 front 0) (place R2 4000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (image PAD (pin P1 1 0 0) (pin P1 2 0 1000))",
            ),
            Some(
                "(logical_part LPA (pin 1 1 G1 0 1 0) (pin 2 2 G1 0 2 0)) \
                 (logical_part LPB (pin 2 1 G1 0 1 0)) \
                 (logical_part_mapping LPA (comp R1)) \
                 (logical_part_mapping LPB (comp R1 R2))",
            ),
            "(net GND (pins R1-1 R2-1)) (class kicad_default GND)",
        );
        assert!(ok);
        let parts: Vec<(&str, usize)> = board
            .logical_parts
            .iter()
            .map(|part| (part.name.as_str(), part.pins.len()))
            .collect();
        assert_eq!(parts, [("LPA", 2), ("LPB", 1)]);
        let lps: Vec<Option<&str>> = board
            .components
            .iter()
            .map(|c| c.logical_part.as_deref())
            .collect();
        assert_eq!(lps, [Some("LPB"), Some("LPB")]);
    }

    /// Jar `/tmp/epic-t8-lrnull.dsn` -> probe FILE 6: the stored null
    /// layer-rule entry kills the parse DURING insert_net_class — AFTER
    /// the class append, the net membership and the valid F.Cu layer rule
    /// already mutated the board. The Rust outcome mirrors the jar NPE:
    /// read_scope false with the partial state observable.
    #[test]
    fn t8_lrnull_deferred_death() {
        let (ok, _state, board) = run_board(
            TWO_LAYERS,
            Some("(component PAD (place R1 2000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (image PAD (pin P1 1 0 0))",
            ),
            None,
            "(net GND (pins R1-1)) \
             (class slow GND \
               (layer_rule F.Cu (rule (width 300))) \
               (layer_rule bogus (width 300)))",
        );
        assert!(!ok, "jar EXCEPTION NPE -> ParseError");
        assert_eq!(board.net_classes.len(), 2, "slow appended before death");
        assert_eq!(board.net_classes[1].name, "slow");
        assert_eq!(
            board.nets[0].net_class, 1,
            "membership applied before death"
        );
    }

    /// Jar `/tmp/epic-t8-emptynet.dsn` -> probe FILE 7: an empty network
    /// scope is a full-tail NOOP — no names seed, no via list change, no
    /// default via infos (the via list is empty), the default rule
    /// creation returns early on zero infos, and the components still
    /// insert.
    #[test]
    fn t8_emptynet_noop() {
        let (ok, _state, board) = run_board(
            TWO_LAYERS,
            Some("(component PAD (place R1 2000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (padstack ViaP (shape (circle signal 600))) \
                 (image PAD (pin P1 1 0 0))",
            ),
            None,
            "",
        );
        assert!(ok);
        assert!(board.nets.is_empty(), "jar NET_COUNT 0");
        assert!(board.via_padstacks.is_empty(), "jar VIA_PADSTACKS []");
        assert!(board.via_infos.is_empty());
        assert!(board.via_rules.is_empty());
        assert_eq!(board.default_via_rule_id(), None);
        assert_eq!(board.net_classes.len(), 1);
        assert_eq!(board.net_classes[0].via_rule, None);
        assert_eq!(board.components.len(), 1);
    }

    /// Jar `/tmp/epic-t8-viadeath.dsn` -> round2 FILE 3: a `(via ...)`
    /// rule whose padstack resolves nowhere fails the dispatch — jar
    /// RESULT ParseError.
    #[test]
    fn t8_viadeath_missing_padstack() {
        let (ok, _state, _board) = run_board(
            TWO_LAYERS,
            Some("(component PAD (place R1 2000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (padstack ViaP (shape (circle signal 600))) \
                 (image PAD (pin P1 1 0 0))",
            ),
            None,
            "(net GND (pins R1-1)) (via V1 NosuchPad default)",
        );
        assert!(!ok, "jar RESULT ParseError");
    }

    /// Jar `/tmp/epic-t8-ordered.dsn` -> round2 FILE 4: `(order ...)`
    /// builds consecutive-pair subnets (sorted, deduplicated) — GND gets
    /// two board nets and the pins land in both as the pairs dictate.
    #[test]
    fn t8_ordered_subnets() {
        let (ok, _state, board) = run_board(
            TWO_LAYERS,
            Some("(component PADI (place R1 2000 2000 front 0) (place R2 4000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (image PADI (pin P1 1 0 0) (pin P1 2 0 1000))",
            ),
            None,
            "(net GND (order R1-1 R2-1 R2-2))",
        );
        assert!(ok);
        let nets: Vec<(&str, i32)> = board
            .nets
            .iter()
            .map(|net| (net.name.as_str(), net.subnet_number))
            .collect();
        assert_eq!(nets, [("GND", 1), ("GND", 2)]);
        let pins: Vec<Vec<i32>> = board
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::Pin { pin, .. } => Some(pin.nets.clone()),
                _ => None,
            })
            .collect();
        // Insertion order R1-1, R1-2, R2-1, R2-2. R1-2 is in NO ordered
        // pair (jar: `PIN cmp=1 idx=1 name=2 ... nets=[]`).
        assert_eq!(pins, [vec![1], vec![], vec![1, 2], vec![2]]);
    }

    /// Jar `/tmp/epic-t8-deg1.dsn` -> round2 FILE 1: a bare `(class slow
    /// GND)` consumes its OWN close as the prevToken seed and breaks on
    /// the network's close — the parse SUCCEEDS and the tail applies slow
    /// (jar NET 1 class=slow, NETCLASS_COUNT 2).
    #[test]
    fn t8_deg1_bare_class_tail() {
        let (ok, _state, board) = run_board(
            TWO_LAYERS,
            Some("(component PAD (place R1 2000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (image PAD (pin P1 1 0 0))",
            ),
            None,
            "(net GND (pins R1-1)) (class slow GND)",
        );
        assert!(ok, "jar RESULT Success");
        assert_eq!(board.net_classes.len(), 2);
        assert_eq!(board.net_classes[1].name, "slow");
        assert_eq!(board.nets[0].net_class, 1);
    }

    /// Jar `/tmp/epic-t8-deg2.dsn` -> round2 FILE 2: the class AFTER the
    /// bare class is swallowed — slow's reader sees `(class` with
    /// prevToken == OPEN and hits its skipScope arm, consuming slow2
    /// whole. jar RESULT Success, NETCLASS_COUNT 2, slow2 gone.
    #[test]
    fn t8_deg2_swallowed_class() {
        let (ok, _state, board) = run_board(
            TWO_LAYERS,
            Some("(component PAD (place R1 2000 2000 front 0))"),
            Some(
                "(padstack P1 (shape (circle F.Cu 400)) (shape (circle B.Cu 400))) \
                 (image PAD (pin P1 1 0 0))",
            ),
            None,
            "(net GND (pins R1-1)) (class slow GND) (class slow2 GND)",
        );
        assert!(ok);
        let names: Vec<&str> = board
            .net_classes
            .iter()
            .map(|class| class.name.as_str())
            .collect();
        assert_eq!(names, ["default", "slow"], "slow2 swallowed");
        assert_eq!(board.nets[0].net_class, 1);
    }

    /// Jar pair battery (`/tmp/epic-t8-pairbat.out`, fixtures
    /// `/tmp/epic-t8-pair*.dsn`): the class-scope clearance pair splitter.
    /// PAIRPROBE `slow_kicad_default` splits into 3 parts and is INERT
    /// (row 2 exists only via the max-fill); FAST `fast_slow` appends
    /// slow-fast + slow-slow; WIRE `wire_via` triggers
    /// createDefaultClearanceClasses (via/smd/pin/area in order) and maps
    /// `wire` to the class's own row; DASH `slow-kicad_default` arrives
    /// pre-split by the reader's dash tokenizer into the INERT `slow` and
    /// the productive `kicad_default` -> slow-kicad + slow-default.
    #[test]
    fn t8_pair_split_rules() {
        let mk = |rule: &str| {
            format!("(net GND (pins A-1)) (class slow GND (rule (clearance 900 (type {rule}))))")
        };
        let cases: Vec<(&str, String, Vec<&str>)> = vec![
            (
                "PAIRPROBE",
                mk("slow_kicad_default"),
                vec!["null", "default", "slow"],
            ),
            (
                "FAST",
                mk("fast_slow"),
                vec!["null", "default", "slow", "slow-fast", "slow-slow"],
            ),
            (
                "WIRE",
                mk("wire_via"),
                vec![
                    "null",
                    "default",
                    "slow",
                    "slow-via",
                    "slow-smd",
                    "slow-pin",
                    "slow-area",
                ],
            ),
            (
                "DASH",
                mk("slow-kicad_default"),
                vec!["null", "default", "slow", "slow-kicad", "slow-default"],
            ),
        ];
        for (name, network, expect_names) in &cases {
            let (ok, _state, board) = run_board(
                &format!("{TWO_LAYERS} (rule (clearance 200))"),
                None,
                None,
                None,
                network,
            );
            assert!(ok, "{name}: jar RES Success");
            assert_eq!(&board.rules.clearance.names, expect_names, "{name}");
            // Every battery shares: (1,1) = 2000 from the structure rule
            // and the slow row/col max-filled to 9000 at append.
            let m = &board.rules.clearance.values[0];
            assert_eq!(m[1][1], 2000, "{name}");
            assert_eq!(m[2][2], 9000, "{name}");
        }

        // PAIRPROBE: no pair writes — the (1,2)/(2,1) 9000s come ONLY from
        // the append max-fill, which writes both matrix sides; INERTness is
        // proven by the names list (no slow-fast/slow-slow rows appended).
        let (ok, _state, board) = run_board(
            &format!("{TWO_LAYERS} (rule (clearance 200))"),
            None,
            None,
            None,
            &mk("slow_kicad_default"),
        );
        assert!(ok);
        let m = &board.rules.clearance.values[0];
        assert_eq!(m[1][2], 9000, "max-fill wrote both sides");
        assert_eq!(m[2][1], 9000, "max-fill wrote both sides");

        // FAST: the pair classes initialized from the slow row, then
        // (slow-fast, slow-slow) = (3, 4) set both ways.
        let (ok, _state, board) = run_board(
            &format!("{TWO_LAYERS} (rule (clearance 200))"),
            None,
            None,
            None,
            &mk("fast_slow"),
        );
        assert!(ok);
        let m = &board.rules.clearance.values[0];
        assert_eq!(m[3][4], 9000);
        assert_eq!(m[4][3], 9000);

        // WIRE: item classes in creation order via/smd/pin/area = 3..6.
        let (ok, _state, board) = run_board(
            &format!("{TWO_LAYERS} (rule (clearance 200))"),
            None,
            None,
            None,
            &mk("wire_via"),
        );
        assert!(ok);
        let items = board.net_classes[1].default_item_clearance_classes;
        // Jar round2-style capture on /tmp/epic-t8-pair-wire.dsn:
        // `items via=3 pin=5 smd=4 area=6` (slot order via,pin,smd,area;
        // the ROW creation order is via,smd,pin,area — see the names list).
        assert_eq!((items[2], items[3], items[4], items[5]), (3, 5, 4, 6));
        // setAll(class_no = 2) fired on the appended class: NONE slot 0
        // stays 0 (the loop starts at i = 1), TRACE slot 1 carries the
        // appended class number.
        assert_eq!((items[0], items[1]), (0, 2));
        let m = &board.rules.clearance.values[0];
        assert_eq!(m[2][3], 9000, "wire->own row vs via");

        // DASH: productive pair is (kicad, default) -> rows 3/4.
        let (ok, _state, board) = run_board(
            &format!("{TWO_LAYERS} (rule (clearance 200))"),
            None,
            None,
            None,
            &mk("slow-kicad_default"),
        );
        assert!(ok);
        let m = &board.rules.clearance.values[0];
        assert_eq!(m[3][4], 9000);
        assert_eq!(m[4][3], 9000);
    }

    // ---- M7-T1: the `(circuit (length max min))` parity pins ------------
    //
    // Java faces (re-verified 2026-09-28 at f32f99bd3):
    // - `Circuit.readLengthScope` (`Circuit.java:64-110`): exactly two
    //   number tokens (Double OR Integer), `lengthArr[0]` -> maxLength
    //   (FIRST number is MAX), `lengthArr[1]` -> minLength; a non-number
    //   aborts the read with an FRLogger warn (log-only — NOT in the
    //   `ReadScopeParameter.warnings` list) and returns null WITHOUT
    //   consuming the rest of the length scope, so the circuit loop then
    //   swallows the trailing tokens and the length scope's `)` closes
    //   the CIRCUIT scope.
    // - `NetClass.readScope` CIRCUIT arm (`NetClass.java:103-112`): the
    //   lengths are ASSIGNED per circuit scope (LAST circuit scope wins)
    //   while use_via/use_layer ACCUMULATE (`addAll`); within one circuit
    //   scope, `Circuit.readScope` re-assigns per `(length` scope (last
    //   wins) and a null read keeps the previous values.
    // - Delivery `Network.insertNetClass` (`Network.java:449-452`): only
    //   `> 0` values are delivered (dsnToBoard-transformed) — the `-1`
    //   "no maximum" sentinel (and 0) stay at the ctor default 0
    //   (`rules/NetClass.java:36-37`). The writer half of the sentinel
    //   contract (`Network.writeCircuit`, `:150-193`: emit
    //   `(length max min)` only when min>0||max>0, max<=0 -> -1,
    //   min<=0 -> 0) has NO Rust counterpart because the Rust workspace
    //   has no DSN writer at all (SES-only) — see the T1 report, the
    //   round-trip face is structurally absent, not divergent.
    // Units: run_board fixes unit=um resolution=10 -> dsn_to_board x10.

    /// The delivery-gate boundary: below (-1, the sentinel), exactly at
    /// (0), and above (1) the `> 0` turn-on; int vs double token
    /// acceptance; the max-first/min-second field order; the x10
    /// transform.
    #[test]
    fn m7_t1_length_boundary_sentinel_order_and_tokens() {
        let (ok, state, board) = run_board(
            TWO_LAYERS,
            None,
            None,
            None,
            "(net NA) (net NB) (net NC) (net ND) \
             (class CLA NA (circuit (length 1 0))) \
             (class CLB NB (circuit (length -1 0))) \
             (class CLC NC (circuit (length 0 0))) \
             (class CLD ND (circuit (length 2.5 0.5)))",
        );
        assert!(ok, "parse Success");
        // Java's circuit warns are FRLogger-only; the warnings LIST stays
        // empty (only Wiring feeds it).
        assert!(state.warnings.is_empty());
        assert_eq!(board.net_classes.len(), 5, "default + CLA..CLD");
        // CLA: max 1 delivered x10, min at the exact boundary 0 gated out;
        // the length delivery does NOT disturb the (empty use_layer)
        // active-layer face — CLA keeps the ctor default.
        assert_eq!(board.net_classes[1].name, "CLA");
        assert_eq!(board.net_classes[1].max_trace_length, 10.0);
        assert_eq!(board.net_classes[1].min_trace_length, 0.0);
        assert_eq!(board.net_classes[1].active_routing_layers, [true, true]);
        assert_eq!(board.net_classes[1].trace_half_widths, [1500, 1500]);
        // CLB: the -1 max sentinel is NOT delivered (stays ctor 0) — an
        // unconditional delivery would land -10 here.
        assert_eq!(board.net_classes[2].name, "CLB");
        assert_eq!(board.net_classes[2].max_trace_length, 0.0);
        assert_eq!(board.net_classes[2].min_trace_length, 0.0);
        // CLC: both tokens at the exact boundary — both gated out.
        assert_eq!(board.net_classes[3].name, "CLC");
        assert_eq!(board.net_classes[3].max_trace_length, 0.0);
        assert_eq!(board.net_classes[3].min_trace_length, 0.0);
        // CLD: double tokens accepted, both delivered x10.
        assert_eq!(board.net_classes[4].name, "CLD");
        assert_eq!(board.net_classes[4].max_trace_length, 25.0);
        assert_eq!(board.net_classes[4].min_trace_length, 5.0);
    }

    /// Last-wins for lengths (across circuit scopes AND across `(length`
    /// scopes in one circuit) versus ACCUMULATION for use_layer — the two
    /// faces the Java circuit arm treats differently in the same
    /// statement block (`NetClass.java:106-111`).
    #[test]
    fn m7_t1_length_last_wins_but_use_layer_accumulates() {
        let (ok, state, board) = run_board(
            TWO_LAYERS,
            None,
            None,
            None,
            "(net N1) (net N2) \
             (class CLM N1 (circuit (length 100 40) (use_layer F.Cu)) \
                        (circuit (length 200 90) (use_layer B.Cu))) \
             (class CLN N2 (circuit (length 100 40) (length 300 70)))",
        );
        assert!(ok, "parse Success");
        assert!(state.warnings.is_empty());
        assert_eq!(board.net_classes[1].name, "CLM");
        // Last circuit scope wins for the lengths...
        assert_eq!(board.net_classes[1].max_trace_length, 2000.0);
        assert_eq!(board.net_classes[1].min_trace_length, 900.0);
        // ...but the use_layer lists ADD UP: both layers active. A
        // last-wins mutant would leave F.Cu inactive with hw 0.
        assert_eq!(board.net_classes[1].active_routing_layers, [true, true]);
        assert_eq!(board.net_classes[1].trace_half_widths, [1500, 1500]);
        // Last `(length` scope wins inside ONE circuit scope.
        assert_eq!(board.net_classes[2].name, "CLN");
        assert_eq!(board.net_classes[2].max_trace_length, 3000.0);
        assert_eq!(board.net_classes[2].min_trace_length, 700.0);
    }

    /// The error face: a non-number where `(length` expects one aborts
    /// THAT read (null, values kept), and the misalignment CASCADES —
    /// each enclosing scope then consumes the next-inner `)` as its own
    /// close: the length scope's `)` ends the CIRCUIT scope, the
    /// circuit's `)` ends the CLASS scope, the class's `)` ends the
    /// NETWORK scope (cursor-verified at f32f99bd3: the closes at bytes
    /// 77/78/79 of the world are consumed by circuit/class/network
    /// respectively), so any class AFTER the failed one is swallowed —
    /// the same degenerate mis-consumption family as
    /// `t8_deg2_swallowed_class`, jar-parity. The parse still SUCCEEDS
    /// (a Close break runs the insertion tail), the earlier length
    /// survives, and the warnings list stays empty (the Java warn is
    /// FRLogger-only).
    #[test]
    fn m7_t1_length_non_number_keeps_previous_and_recovers() {
        let (ok, state, board) = run_board(
            TWO_LAYERS,
            None,
            None,
            None,
            "(net NE) (net NF) \
             (class CLE NE (circuit (length 100 40) (length X 9))) \
             (class CLF NF (circuit (use_layer B.Cu)))",
        );
        assert!(ok, "parse recovers");
        assert!(state.warnings.is_empty(), "FRLogger warn is log-only");
        // The failed second read kept the first `(length 100 40)`.
        assert_eq!(board.net_classes[1].name, "CLE");
        assert_eq!(board.net_classes[1].max_trace_length, 1000.0);
        assert_eq!(board.net_classes[1].min_trace_length, 400.0);
        // CLF is swallowed by the mis-consumption cascade (Java parity):
        // the network scope ends at CLE's own closing bracket.
        assert_eq!(board.net_classes.len(), 2);
        assert_eq!(board.net_classes[1].min_trace_length, 400.0);
    }

    /// The no-circuit face stays byte-stable at the ctor zeros: a class
    /// without any circuit scope, and a class whose circuit scope carries
    /// only `(use_layer ...)` (the independent face) — both keep
    /// min/max 0; the use_layer delivery (active layers + the inactive-hw
    /// zeroing) is unaffected by the length absence.
    #[test]
    fn m7_t1_no_circuit_and_lengthless_circuit_default_to_zero() {
        let (ok, state, board) = run_board(
            TWO_LAYERS,
            None,
            None,
            None,
            "(net NG) (net NH) \
             (class CLG NG (pull_tight on)) \
             (class CLH NH (circuit (use_layer F.Cu)))",
        );
        assert!(ok, "parse Success");
        assert!(state.warnings.is_empty());
        assert_eq!(board.net_classes[1].name, "CLG");
        assert_eq!(board.net_classes[1].max_trace_length, 0.0);
        assert_eq!(board.net_classes[1].min_trace_length, 0.0);
        assert_eq!(board.net_classes[2].name, "CLH");
        assert_eq!(board.net_classes[2].max_trace_length, 0.0);
        assert_eq!(board.net_classes[2].min_trace_length, 0.0);
        // The lengthless circuit still delivers its use_layer face:
        // B.Cu inactive (and its hw zeroed by the inactive-hw pass).
        assert_eq!(board.net_classes[2].active_routing_layers, [true, false]);
        assert_eq!(board.net_classes[2].trace_half_widths, [1500, 0]);
    }
}
