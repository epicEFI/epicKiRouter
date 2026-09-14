# EpicRouter M1b: DSN Parser + SES Writer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the Specctra DSN reader (`DsnReader.readBoard`) and SES writer (`SesWriter.write`) to `rust/crates/epic-dsn`, proven by two differential gates against the frozen Java jar: a **parse-parity digest** on 175 fixtures (+1,157-file soak) and an **SES canonical compare** on the tier fixtures — both driven by committed goldens, java-free in CI.

**Architecture:** Transliteration of `io.specctra` (≈13.5k Java LOC incl. ~1.15k JFlex DFA tables that become a hand-written lexer). The parser builds a **parse-derivable mini board model** (`SesBoard`) behind a `BoardSink` trait (the Rust equivalent of Java's `BoardParserCallback` seam) — no normalization, no contact sets, no router types. SES emission runs off that mini model. Byte-parity of SES is *not* an M1b gate (it depends on M2 board machinery); it is a tracked diagnostic that must be fully classified.

**Tech Stack:** Rust 1.93 workspace; `epic-dsn` depends ONLY on `epic-geometry` (+ workspace deps). Oracle: single-file Java launchers under `rust/harness/oracle/` run against the fat jar (JDK 25), batched JSONL, one JVM process. serde/serde_json for harness-side digest types only (crate stays pure per M1a D3).

**Design doc:** `docs/superpowers/specs/2026-09-11-epicrouter-rust-rewrite-design.md` (§6 M1 row; note its "162 fixtures" number is stale — see Task 14 Step 4).

**Standing constraints (every task):**
- Never modify Java sources under `src/` or `src_v19/`. Never touch other Claude instances or global `~/.claude`. Never push to any remote.
- Commits on `epic/main`, trailer `Co-Authored-By: Claude Code <noreply@anthropic.com>`.
- Oracle launchers live in `rust/harness/oracle/` — never `src/`.
- `cargo clippy --workspace --all-targets -- -D warnings` green (incl. `clippy::unwrap_used`; tests use `.expect()`).
- Run cargo from `rust/`; run git from the repo root.
- Oracle pins are jar-captured (jshell/launcher), never memory-reconstructed; non-degenerate; branch-executing (cerebrum rule, 2026-09-12).

---

## Ground truth from recon (established 2026-09-13; do not re-derive)

### io.specctra at a glance

Outer package `src/main/java/app/freerouting/io/specctra/` — 7 files, 1,843 LOC. Grammar `parser/` — 39 files, 11,405 LOC (1,862 of which are the JFlex-generated tokenizer `SpecctraDsnStreamReader`, ~1,150 lines of packed DFA tables). Support in `io/`: `CoordinateTransform` (175), `BoardReadResult` (56, sealed Success|OutlineMissing|ParseError|IoError), `BoardMetadata` (34).

Key entries: `DsnReader.readBoard(InputStream, BoardObservers, IdGenerator[, designName]) -> BoardReadResult` (`DsnReader.java:58`), `DsnReader.readMetadata` (`:182`, early-stop path), `SesWriter.write(BasicBoard, OutputStream, String)` (`SesWriter.java:55-67`). Both public static → launcher-drivable.

### DSN → board flow (headless)

1. `readBoard` hand-scans exactly 3 tokens `( pcb <name>` then `yybegin(NAME)` (`DsnReader.java:83-109`); garbage → `ParseError`.
2. `Keyword.PCB_SCOPE.readScope` base loop dispatches on `nextToken instanceof ScopeKeyword`; **anything else is skipScoped** (`ScopeKeyword.java:46-81`) — this kills `(unit ...)` (T24).
3. Board created mid-parse: `Structure.readScope → createBoard` (`Structure.java:1139-1286`) computes the scale factor (T25), builds rules/clearance matrix, calls the `BoardParserCallback.createBoard` seam (`:1268-1274`), inserts outline holes as keepouts.
4. Post-create in Structure order: flip_style, keepouts, planes → conduction areas, `insertMissingPowerPlanes`, autoroute-settings handoff (`Structure.java:1041-1134`).
5. Components/nets: `Network.readScope → insertComponents` in parse order (`Network.java:832-837`); net numbers sequential in `(net ...)` file order (`:1414`).
6. Wiring inserts in file order; Java ends with `board.normalizeAllTraces()` (`Wiring.java:345-353`) — **NOT ported** (board machinery, M2; documented divergence class).
7. Post-read: `DsnFile.adjustPlaneAutorouteSettings` when no `(autoroute ...)` scope (`DsnReader.java:133-136`, `DsnFile.java:32-114`) — ported (parse-derivable, T37).
8. Headless-only side effects (copper-to-edge override, hole-keepout class) are CLI-gated no-ops for a plain parse — out of scope.

### Parity traps T24–T46 (each needs a pinned test, a corpus case, or a golden-observed value)

| # | Trap | Java behavior to reproduce |
|---|---|---|
| T24 | `(unit ...)` scope is dead | No UNIT keyword exists; pcb-level loop skips non-ScopeKeyword tokens (`ScopeKeyword.java:65-77`); `Unit.readScope` returns false (`Unit.java:33-35`). Unit/resolution come only from `(resolution <unit> <n>)` (`Resolution.java:28-72`); defaults MIL/100 (`ReadScopeParameter.java:87-89`). A `(unit mm)`-only file parses as MIL/100. |
| T25 | Scale-factor overflow loop | `Structure.java:1189-1203`: `scaleFactor = max(resolution,1)`; `while (5*maxCoor >= CRIT_INT) { scaleFactor/=10; maxCoor/=10; }` — integer division loop; output granularity feeds every coordinate. |
| T26 | Two float parsers | `nextDouble()` = `NumberFormat.getInstance(Locale.US)`, null on failure (`SpecctraDsnStreamReader.java:1829-1840`); `Package.readRotation` = `Double.parseDouble` (`Package.java:339`). Port per call site. |
| T27 | `nextString` buffer step-back | Stops at space/CR/LF/`(`/`)`, honors quote char, steps back one char if the string ended on a bracket (`SpecctraDsnStreamReader.java:1739-1796`, bracket `:1786-1792`). |
| T28 | `nextStringList` swallows empty first element | KiCad 8 netlist workaround (`SpecctraDsnStreamReader.java:1805-1812`). |
| T29 | Lexical-state name/keyword ambiguity | 8 lexical states (`:29-37`); `via` → Keyword only from initial-ish states; names force NAME state (`:948-951`); `skipScope` forces NAME (`ScopeKeyword.java:24`); a keyword-looking layer name is a String in NAME state. |
| T30 | warn-vs-error asymmetry | Missing via padstack `Wiring.java:661-671`, unknown-layer wire dropped `:459-470`, degenerate keepout `Structure.java:862-876` → warn+continue; others fatal. Warnings list is parity-observable (`DsnReader.java:137-146`). |
| T31 | Degenerate-input skips | zero-area keepouts (`Structure.java:862-876`), zero-length traces (`Wiring.java:516-545`), duplicate vias (`:699-703`), wires-outside-bbox dropped (`:498-510`), `(path)` with <5 tokens → null (`Shape.java:470-478`). Task 6 jar corrections: the zero-area-keepout warning is **FRLogger log-only** — it does NOT enter the D12 digest warnings list (that list is fed only by `Wiring.java`'s `warnings.add` sites via `ReadScopeParameter.warnings`); and nonzero-radius **circle keepouts INSERT** (`Circle.dimension()==2` for radius>0, `Circle.java:36-42`) — only radius-0 circles are degenerate. |
| T32 | Padstack-name `.N` suffix strip | `replaceAll("\\.\\d+","")` at `Wiring.java:659` and `Network.java:1289-1291`; ALSO at `Library.java:113` (strip happens BEFORE the dedup `get` at `:158`, so library dedup is effectively strip-match — jar-probed Task 4). Subnet number is a bare positional int after the net name (`Network.java:1336-1345`), not a scope. Task 4 fix-round correction: the network-TAIL via-name clean is this same `.N`-strip (`Network.java:1292-1294`); the via-INFO path's registry get (`:275`) does NOT strip. |
| T33 | Clearance-class pair 3-way syntax | 2-element list, quote-split, underscore-split + `smd_to_turn_gap` special (`Structure.java:712-747`); `wire`→class 1 (`:750-763`). |
| T34 | Electra Top/Bottom fallback | `LayerStructure.getNo`: any name containing "Top" → 0, "Bottom" → last (`LayerStructure.java:33-40`). |
| T35 | Net lookup case-insensitive vs netlist case-sensitive | `rules/Nets.java:39,51` equalsIgnoreCase vs `parser/NetList.java:15` TreeMap on case-sensitive `Net.Id.compareTo` (`parser/Net.java:96-102`); numbers by file order (`Network.java:1414`). |
| T36 | `(type route)` → USER_FIXED | `Wiring.calcFixed` maps any token ≠ shove_fixed/fix/normal to USER_FIXED (`Wiring.java:257-278`); KiCad pre-routes export as `(type protect)` (`SesWriter.java:478-490`). **Verify with jar spike in Task 1 before pinning** (hypothesis is code-derived). |
| T37 | `adjustPlaneAutorouteSettings` heuristic | `DsnFile.java:32-114`: >50% board area, middle signal layer, no traces → plane + USER_FIXED bump; runs when no `(autoroute ...)` scope. |
| T38 | `IdentifierType.write` de-quote off-by-one + quoting | `substring(1, len-2)` in the de-quote loop (`IdentifierType.java:21` — drops TWO trailing chars); quote on reserved chars / non-ASCII signed bytes / leading digit (`:31-49`); SES reserved set `{"(",")"," ",";","-","_","/","~","{","}"}` (`SesWriter.java:62`). |
| T39 | SES item order = DESCENDING item id | `UndoableObjects` = ConcurrentSkipListMap keyed by item; `Item.compareTo = other.id - id` (`UndoableObjects.java:36-40,315-317`, `Item.java:95-103`); placement/components ascending (`SesWriter.java:126`); library dedup first-name-wins (`:243-252`). |
| T40 | Endpoint snapping before emission | `snappedEndpoint` (`SesWriter.java:426-453`) — depends on contact sets. **NOT ported in M1b**; documented divergence class in the byte-diff report. |
| T41 | `Math.round` + double printing | SES coords = `(int) Math.round(...)` (`SesWriter.java:163-169,399-407`) — use M1a `java_round`; `String.valueOf(double)` for DSN doubles; rotation 3-decimal trim (`:259-269`). |
| T42 | `readMetadata` early-stop | `DsnReader.java:182-280` parses only parser/resolution/structure then breaks (`:243-249`) — second divergent path; pin shared semantics with readBoard. |
| T43 | Bounds offset(+1000) + dsnToBoard re-derivation | `Structure.java:1207-1208`; SES scaleFactor = `coordinateTransform.dsnToBoard(1) / resolution` (`SesWriter.java:80-82`). |
| T44 | `Polygon.boundingBox` offset paren bug | x-max: `Math.max(...) + offset` (OUTSIDE max) vs y-max inside (`PolygonPath.java boundingBox()`). Bug-compatible port. |
| T45 | TreeMaps reorder pin/keepout infos | `Component.java:190-194` — name-sorted, not file order; observable in pin insertion order. |
| T46 | KiCad-header stress | quoted names with spaces (`sonde xilinx`), apostrophes in host_cad (`Parser.readQuoteChar`, `Parser.java:147-168`), um/10 resolution, `(property (index N))` skipped generically (`Structure.java:394-395`). |
| T47 | `(class ...)` before `(wiring ...)` discards all wires | Jar-verified in Task 1 (commit `502b88f8`): a `(class ...)` scope appearing before `(wiring ...)` makes the Java reader silently drop every wire (0 traces, NO warning); with wiring first, insertion works. Pin in Task 8 with a crafted pair fixture; keep bug-compatible in M1b. **Reconciled (spec review 2026-09-14, `/tmp/epic-t9fix-t47.jsh` → `/tmp/epic-t9fix-t47.out`)**: on the composed shape (bare class inside network + wiring after it, closes balanced) Java and Rust AGREE — Success, 0 traces, no warnings, jar consumes all 748 bytes (tokens past `(wiring` at offset 659); the "drop" is the network scope's bare-class close-eating cascade (`t8_deg1`/`t8_deg2` in `scope/network.rs`), NOT a wiring veto — the valid wire would have inserted (or warned per T30/T31) had the wiring scope dispatched. Equivalence pinned at the dispatcher level by `t47_class_before_wiring_cascade_parity` in `rust/crates/epic-dsn/src/reader.rs`. |
| T48 | Plane/keepout hole failures are ORDER-SENSITIVE (first failure wins) | Jar-verified in Task 6 fix round (`a5d77902`, `/tmp/epic-t6-nullhole.out`): `transformAreaToBoard` hole loop — a Circle/null-transform hole at i=0 returns null (plane skipped, parse SURVIVES via `BasicBoard.insert_conduction_area` log-warn) BEFORE a null hole ENTRY at i=1 is ever reached; null entry first → NPE, parse dies. Same two windows, opposite order, opposite outcomes. Port models this as three-state `TransformedArea` (`Area`/`Null`/`Threw`) with first-failure-wins order; both flavors fatal at keepout site. |
| T49 | Image `(keepout ...)` scopes are component-relative | Java applies the placement transform LAZILY in `ObstacleArea.getArea()` (`ObstacleArea.java:119-144`) — the item stores image-relative geometry plus `translation`/`rotationInDegree`/`sideChanged`, and consumers see: mirrorVertical(0,0) if sideChanged && !flipRotateFirst → rotation (exact multiples via `turn90Degree((int)rot/90, 0)`; anything else via `rotateApprox(toRadians(rot), FloatPoint 0)` with rounding BEFORE any later mirror) → mirrorVertical if sideChanged && flipRotateFirst → translateBy(translation). flipRotateFirst = `board.components.getFlipStyleRotateFirst()` (`BasicBoard.java:73`), set by `(flip_style rotate_first)`. Order-sensitivity is REAL, not academic: on dsn-0033 keepout id=1479 (rel circle −55000,0 r 9000, rot 330°, side-changed) the default order gives (1695289,−1314250) while rotate-first gives (1790551,−1314250) (jar `/tmp/epic-t49b.jsh`). Jar-verified branch-complete in Task 12 (`/tmp/epic-t12-probe.out`, 34 branch classes over the 175-fixture corpus; beware mirror-degenerate examples where center.x=0 or a mirror-symmetric polygon hides the mirror, and note the polygon branch RE-NORMALIZES corner order under the transform — pin the dumped sequence, never a hand permutation). Port: `BoardShape::mirror_vertical/turn_90_degree/rotate_approx/translate_by` + `SesBoard::obstacle_absolute_area`, applied in the digest keepout arm; pinned by `t49_obstacle_absolute_area_matches_jar_captures` (11 rows, `ses_board.rs` tests). |

### Port map (Java → `epic-dsn/src/`)

| Java | Rust module | Responsibility |
|---|---|---|
| `SpecctraDsnStreamReader` | `lexer.rs` | hand-written tokenizer: 8 lexical states, token kinds, `next_string`/`next_string_list`/`next_double` semantics (T26–T29) |
| `Keyword`, `ScopeKeyword` | `keyword.rs` | token enum + `skip_scope` |
| `DsnReader` | `reader.rs` | `read_board(&[u8]) -> DsnReadResult` + `read_metadata` (T42) |
| `ReadScopeParameter` | `state.rs` | mutable parse state |
| `Parser.java` | `scope/parser_scope.rs` | `(parser ...)` incl. quote char |
| `Structure.java` | `scope/structure.rs` | layers/boundary/vias/rules/keepouts/planes + createBoard via sink |
| `Placement`, `Component`, `PlaceControl` | `scope/placement.rs` | placements (T45) |
| `Library`, `Package`, `PartLibrary` | `scope/library.rs` | padstacks/images/logical parts |
| `Network`, `NetClass`, `Net`, `NetList`, `Circuit` | `scope/network.rs` | nets/classes/class_class/via infos (T32, T35) |
| `Wiring` | `scope/wiring.rs` | wires/vias/conduction areas (T30, T31, T36) |
| `AutorouteSettings` | `scope/autoroute_settings.rs` | raw settings IR (D15) |
| `Shape` + leaf shape classes | `shape.rs` | shape IR + `transform_to_board[_rel]` (T44) |
| `LayerStructure`, `Layer` | `layer_structure.rs` | layer table (T34) |
| `io/CoordinateTransform` | `coordinate_transform.rs` | resolution scaler |
| `BoardParserCallback` (seam) + mini model | `sink.rs`, `ses_board.rs` | `BoardSink` trait; `SesBoard` parse-derivable board (D9) |
| `WriteScopeParameter`, `IndentFileWriter`, `IdentifierType` | `write_scope.rs` | emission core (T38) |
| `SesWriter` | `ses/writer.rs` | session emission (T39, T41) |
| **Deferred (D8)** | — | `DsnWriter`, `SesReader`, `SessionToFusion`, `RulesReader`, `RulesWriter` |

Dependency rule (ArchUnit equivalent, `SpecctraPackageArchTest.java:33-43`): `epic-dsn` depends on `epic-geometry` only — never on future board/router/engine crates.

### Fixture inventory (verified 2026-09-12)

- Tier fixtures: 23 (A=11, B=9, C=3) at `scripts/benchmark/fixtures` per `rust/harness/config/tiers.yaml:10-39`.
- Root fixtures: **152 `.dsn`** + 39 `.ses` + 13 `.rules` under `fixtures/` (16.3 MB) — includes known-issue stress files (Issue199/015 StackOverflow deep nesting, Issue035, Issue180, Issue191, Issue269).
- Benchmark corpus: 2,334 `.dsn` (134.7 MB); the 1,157 `unrouted.dsn` files are the soak set.
- Digest-parity set = 23 tier + 152 root = **175 fixtures**; soak = 1,157 (D10).
- Feature coverage in corpus: planes 574 files, keepouts 1,091, net classes 2,334, `(window)` holes 16, KiCad um/10 + quoted names in tier A.

### Digest schema + oracle mechanics

`rust/harness/oracle/DsnParseOracle.java` (~200 lines, batched JSONL, per-case flush, unknown-op → exit 3 with id on stderr — M1a conventions). Per fixture, one JSON line:

```json
{"id":"fix-0007","file":".../example.dsn","result":"Success",
 "stats":{"layers":4,"items":1201,"components":58,"pads":412,"nets":31,"traces":900,"vias":45},
 "geometry_sha256":"...","clearance":{"classes":["default","power"],"values":[[0,25400],[25400,0]]},
 "net_table":["GND","VCC",...],"layer_table":[{"name":"F.Cu","signal":true},...],
 "warnings_n":["keepout skipped in DsnFile ..."],"unit":"UM","resolution":10,"snap_angle":"45"}
```

- `geometry_sha256`: SHA-256 over canonical text, one line per item in **descending id** (T39 order): `T <id> <layer> <halfwidth> <x0> <y0> <x1> <y1>...`, `V <id> <padstack> <x> <y> <fixed>`, `K <id> <layer> <class> <shape-kind> <coords>`, `A <id> ...` (conduction area). Ints decimal; DSN-sourced doubles as `bits` i64 (M1a T12).
- `warnings_n`: exact warning strings with every digit sequence replaced by `#` (D12).
- Rust side mirrors field-for-field; harness subcommands `dsn golden` (needs jar+JDK) / `dsn compare` (java-free, CI-able), goldens committed at `rust/harness/corpus/dsn-golden.jsonl`.

### SES determinism audit (recon §4, verified)

Writer has NO timestamps/RNG/UUID; ordering fully determined (placement by id asc, items by id desc, padstack first-name-wins dedup, nets 1..max); encoding fixed (UTF-8, LF, 2-space indent, `IndentFileWriter.java:12,19,55`). The only non-parse-derivable behaviors are `snappedEndpoint` (T40) and the parse-time `normalizeAllTraces()` — both M2 board machinery. Hence D11.

---

## Locked decisions (do not relitigate during execution)

- **D8 — Scope:** M1b ports `DsnReader.readBoard` (+`readMetadata`) and `SesWriter.write`. Deferred and ledgered: `DsnWriter`, `SesReader`, `SessionToFusion` (M2), `RulesReader`/`RulesWriter` (M3, settings work). The design's round-trip gate needs none of the deferred pieces.
- **D9 — BoardSink seam + SesBoard mini model:** the parser emits through `trait BoardSink`; `SesBoard` is the parse-derivable concrete sink (layers, nets, placements, padstack registry, sequential-id items, clearance IR, warnings, metadata, transform). NO normalization, NO contact sets, NO router types. epic-board (M2) will implement the same trait against the real model.
- **D10 — Parse-parity gate:** digest goldens for all 175 digest fixtures, committed; soak = 1,157 benchmark `unrouted.dsn` with Java-captured digests too. CI runs `dsn compare` java-free over the full set if it completes <60 s in CI; otherwise a `--set digest` default (175) with `--set soak` opt-in — measure in Task 11 and record the choice here. **Adjudicated 2026-09-14 (Task 11):** release-build measurements on the dev machine — `dsn compare --set all` (1,332 fixtures, java-free) 2.6 s; `dsn golden --set all` (one JVM) 19.7 s. With the ~2x CI margin the full compare is ≈5 s ≪ 60 s, so CI runs the FULL set (`dsn compare`, all 1,332); no digest-only default needed. (Task 12 note: `dsn compare` intentionally exits 1 on the known pre-parity mismatches until Task 12 lands parse parity.)
- **D11 — SES gate:** canonical compare = sexpr token-tree equality (whitespace/indent-insensitive, numbers exact, strings exact) between Java's parse→emit .ses and Rust's, on tier A (11) + tier B (9). Byte-equal count is a **tracked diagnostic, not a gate**: every byte diff on tier A must be classified as T40-snapping / normalization / genuine bug; zero unclassified at exit. Byte-parity becomes a hard gate in M2.
- **D12 — Warnings parity:** digit-normalized strings (`[0-9]+` → `#`) + count. Exact raw strings are captured in the golden but not compared.
- **D13 — Numbers in goldens:** ints decimal, doubles as `bits` i64 (M1a T12), floats `fbits` i32; `Infinity` token convention inherited.
- **D14 — Oracle invocation:** batched JSONL, ONE JVM process per golden run, per-case flush (M1a bug-078-class lesson: no interleaving corruption).
- **D15 — AutorouteSettings → raw IR struct** in epic-dsn (field-for-field parse result); the RouterSettings merger is M3 (epic-engine).
- **D16 — `readMetadata` is ported** with a shared-semantics pin: on all 175 fixtures, `read_metadata` fields must equal the fields derivable from `read_board` (Java-side verified once via the oracle; Rust-side asserted in tests).

---

## Task 1: Crate scaffold + tokenizer (lexer, keyword, scope-skip) + T36 jar spike

**Files:** Modify the M0 stubs `rust/crates/epic-dsn/src/lib.rs` + `Cargo.toml` (add the epic-geometry path dep; crate + workspace membership already exist from M0); create `src/lexer.rs`, `src/keyword.rs`.

- [ ] **Step 1 — T36 jar spike (BEFORE writing pins):** run jshell against the jar (`~/.jdks/jdk-25.0.4.1+1/bin/jshell --class-path build/libs/freerouting-current-executable.jar` from repo root). Feed a minimal DSN with `(wiring (wire ... (type route)))` through `DsnReader.readBoard` (null observers, null id generator; wrap bytes in ByteArrayInputStream), then inspect the resulting trace's fixed state via reflection (`getDeclaredMethod`/`setAccessible(true)` as in M1a) or by writing the SES and checking for `(type protect)`. Record the ACTUAL behavior in a comment in `scope/wiring.rs` (created Task 8 — put the transcript in the Task 1 commit message or a doc comment in keyword.rs temporarily). If the hypothesis (USER_FIXED) is wrong, update T36 in THIS plan (doc-of-record) in the same commit.
- [ ] **Step 2 — Failing tests for the lexer core.** The lexer ports `SpecctraDsnStreamReader` semantics, NOT its DFA: hand-written scanning with the 8 lexical states (`SpecctraDsnStreamReader.java:29-37`), token kinds String/Integer/Double/Keyword/open+close bracket/EOF. Tests (values jar- or spec-pinned where numeric):
  - `next_string` stops at ` `/CR/LF/`(`/`)`, honors quote char, steps back on trailing bracket (T27) — pin: input `(net GND 1)` read state by state.
  - `next_string_list` drops empty first element (T28) — pin with `("" foo bar)` shape from KiCad 8 netlists.
  - `next_double` = NumberFormat(Locale.US) lenient parse returning None on failure (T26) vs `read_rotation` = strict parse_double — pin both on `"1.5e2"`, `"1.5x"` (NumberFormat parses prefix `1.5`, strict fails), `".5"`.
  - integer tokens via `Integer.valueOf` semantics incl. overflow → what Java does (`Integer.valueOf("2147483648")` throws → scanner behavior: pin what the jar actually does with a huge int token).
  - lexical states (T29): `via` recognized as Keyword from initial state but as String in NAME state; `skip_scope` forces NAME state.
- [ ] **Step 3 — Implement lexer + `Keyword` enum + `ScopeKeyword::skip_scope`** (skip until matching close, forcing NAME state — `ScopeKeyword.java:24`).
- [ ] **Step 4 — Gates + commit** `feat(m1b): epic-dsn scaffold + hand-written DSN tokenizer (8 lexical states)`.

## Task 2: Coordinate transform, resolution/unit, layer structure, parse state

**Files:** Create `src/coordinate_transform.rs`, `src/layer_structure.rs`, `src/state.rs`; modify lib.rs.

- [ ] Port `io/CoordinateTransform.java` (dsn↔board scaling, both directions), `Resolution`/`Unit` (T24: unit ONLY from `(resolution ...)`; defaults MIL/100 — pin `(unit mm)`-only file parsing as MIL/100).
- [ ] Scale-factor loop T25: transliterate `Structure.java:1189-1203` integer-division `while` exactly; pin from the jar (jshell: feed resolutions 100/10/1 with huge coordinates; capture the resulting scaleFactor) — at least 3 jar-captured pins including one that enters the loop.
- [ ] `LayerStructure` with Electra fallback (T34): name containing "Top" → 0, "Bottom" → last; pin `get_no("TopLayer")`, `get_no("bottom copper")`, exact-name wins over substring.
- [ ] `state.rs`: `ParseState` struct — unit, resolution, warnings `Vec<String>`, plane list, placement list, netlist (case-SENSITIVE TreeMap order for parse, T35), host metadata; no logic beyond defaults (pin defaults MIL/100).
- [ ] Gates + commit `feat(m1b): coordinate transform + resolution + layer structure (Electra fallback)`.

## Task 3: Shape IR + transform_to_board

**Files:** Create `src/shape.rs`; uses epic-geometry (`PolygonShape`, `PolylineArea`, `IntOctagon::enlarge`, `FloatPoint::bounding_octagon` — all public since M1a).

- [ ] Shape enum `{ Rectangle, Polygon(PolygonPath), Circle, PolylinePath, Path }` mirroring `Shape.java` dispatch + leaf classes; `(path ...)` with <5 tokens → None (T31, `Shape.java:470-478`).
- [ ] `transform_to_board` / `transform_to_board_rel` via `CoordinateTransform` — transliterate `Shape.java:447-516` (`PolygonPath` reader is the hot one for reference-routed wires).
- [ ] `Polygon.boundingBox` paren bug T44: x-max `Math.max(...) + offset` OUTSIDE, y-max inside — bug-compatible; pin with a polygon where the two orderings differ (offset ≠ 0, max at last vertex) using jar-captured values.
- [ ] Gates + commit `feat(m1b): shape IR + board transform (polygon bbox paren bug kept)`.

## Task 4: BoardSink seam + SesBoard mini model + digest skeleton

**Files:** Create `src/sink.rs`, `src/ses_board.rs`; harness: `rust/harness/src/dsn_digest.rs`.

- [ ] `trait BoardSink` mirroring `BoardParserCallback` (`parser/BoardParserCallback.java:42`): `create_board(...)`, plus insertion methods the scope readers drive (keepout, plane→conduction area, trace, via, pin/component, net, clearance class...). Method-per-insertion, NO board intelligence in the trait.
- [ ] `SesBoard` (D9): layers, net table (number order), placements (insertion order + ids), padstack registry (insertion order, first-name-wins alias after `.N` strip T32), items `Vec<ItemIr>` with sequential ids (Trace/Via/Keepout/ConductionArea IRs), clearance IR, warnings, metadata, transform. Item id assignment mirrors the Java `IdGenerator` (sequential ints — verify the actual generator in the jar spike; capture first/last id on one fixture).
- [ ] Canonical geometry text + SHA-256 (`dsn_digest.rs` in harness; the canonical-line formats from the digest schema above, descending id order T39).
- [ ] Gates + commit `feat(m1b): BoardSink seam + SesBoard parse-derivable model + digest skeleton`.

## Task 5: Structure scope I — createBoard, boundary, rules, clearance

**Files:** Create `src/scope/structure.rs`.

- [ ] `createBoard` flow: scale factor (calls Task 2), `BoardRules` + clearance matrix IR construction via sink, bounds = bbox transform + offset(1000) (T43, `Structure.java:1207-1208`), outline holes → keepouts.
- [ ] Boundary scope incl. `clearance_class` parse (Issue558 context: parses, defaults to AREA class — pin).
- [ ] Clearance-class pair 3-way syntax (T33, `Structure.java:712-747`) + `smd_to_turn_gap` + `wire`→class 1; pin all three syntax variants jar-captured.
- [ ] Rule/layer_rule scopes (`Rule.java` 343 LOC): width/clearance into IR.
- [ ] Gates + commit `feat(m1b): structure scope I — createBoard seam, boundary, clearance classes`.

## Task 6: Structure scope II — keepouts, planes, power-plane heuristic

**Files:** Modify `src/scope/structure.rs`.

- [ ] Keepout/place-keepout/via-keepout scopes with degenerate skips (T31: zero-area dropped with warning `Structure.java:862-876` — warning is FRLogger log-only, NOT D12 digest surface; see T31 row. NONZERO-RADIUS CIRCLE KEEPOUTS INSERT — `Circle.dimension()` is 2, jar-refuted in Task 6).
- [ ] Plane scopes → conduction areas (`Structure.java:1068-1122`) + `insertMissingPowerPlanes` (`:526-571`). IR decision (Task 3 quality review): represent `PolylineArea` as a dedicated `BoardArea { border, holes }`-style IR — `BoardShape` alone cannot carry holes; design it in here, not bolted on.
- [ ] `adjust_plane_autoroute_settings` heuristic (T37, `DsnFile.java:32-114`): >50% area, middle signal layer, no traces → plane + USER_FIXED; runs only when no `(autoroute ...)` scope. Pin on a crafted fixture where the heuristic fires AND one where it doesn't (both jar-captured via digest warnings/fixed states).
- [ ] Flip_style + snap angle.
- [ ] Gates + commit `feat(m1b): structure scope II — keepouts, planes, power-plane heuristic`.

## Task 7: Placement + Library/Package/PartLibrary

**Files:** Create `src/scope/placement.rs`, `src/scope/library.rs`.

- [ ] `(placement (component ... (place ...)))` per `Component.java` incl. TreeMap-sorted pin/keepout infos (T45 — pin insertion order differing from file order on a crafted component), rotation via strict parse (T26).
- [ ] `(library ...)`: padstacks (circle/rect/polygon shapes, `Library.java` 473 LOC), `(image ...)` pin definitions (`Package.java` 425), placement rotation helper. NOTE — Task 2's claim that `Package.java:175` passes the relative pin location through the base-ADDING `boardToDsn(FloatPoint[])` was REFUTED in Task 7 (spec review confirmed on static types): `:175` passes `relativeLocation`, declared type `Vector`, resolving to `boardToDsn(Vector)` (`CoordinateTransform.java:71-77`) — pure scaling, NO base shift. No quirk; do not add one in Task 13.
- [ ] `(part_library ...)` logical parts/mappings (`PartLibrary.java` 351) → IR.
- [ ] KiCad stress pins (T46): quoted name with space, apostrophe quote char (`Parser.readQuoteChar`), `(property (index N))` skipped — use real tier-A file bytes in tests.
- [ ] Gates + commit `feat(m1b): placement + library/package/part-library scopes`.

## Task 8: Network scope — nets, classes, via infos

**Files:** Create `src/scope/network.rs`.

- [ ] `(network ...)` per `Network.java` (1,465 LOC — the biggest reader): nets/pins, net classes + class_class, via infos/rules, logical parts, component insertion in parse order (`:832-837`). Via-list replace semantics (jar-pinned Task 4, `/tmp/epic-t4c-images.out`): ANY `(class ...)` scope makes `viaPadstackNames` non-null — even with empty `use_via` — and the tail replace then WIPES the via list to empty (`Network.java:1286-1313`; NetClass has no bare `(via ...)` keyword, `NetClass.java:96-121`/`Circuit.java:52-53`). Sink API: `set_via_padstacks` (replace) vs `append_via_padstack` (dedup+append) — the tail replace clobbers earlier via-info appends.
- [ ] Net numbering: sequential in `(net ...)` file order (`:1414`); lookup case-insensitive vs netlist case-sensitive TreeMap (T35) — pin a fixture with `GND` + `gnd` distinct at parse, merged at lookup, jar-captured digest.
- [ ] Padstack `.N` strip (T32) on via padstack resolution.
- [ ] `(circuit ...)` length matching → IR.
- [ ] Gates + commit `feat(m1b): network scope — nets, classes, class_class, via infos`.

## Task 9: Wiring scope + autoroute settings + parser scope + read_board assembly

**Files:** Create `src/scope/wiring.rs`, `src/scope/autoroute_settings.rs`, `src/scope/parser_scope.rs`; modify `reader.rs`.

**Part A — Task 8 quality-review carryovers (commit separately, before Part B):** (1) `contains_wire_clearance_pair` dead duplicate at `scope/network.rs:1270` — import the `pub(crate)` copy from `scope/structure.rs:1252` instead; (2) comments misdescribe Java in the class-scope string arms (`network.rs:366-370`, `:387`, module doc `:86-91`) — Java STORES the null for `(via_rule ...)` (`NetClass.java:100-101`); only `(clearance_class ...)`'s null is parse-fatal (`NetClass.java:110-114`), and the Rust side folds that to `""` per the accepted Task-5 scanner-null fold — describe both accurately so no one "fixes" parity in the wrong direction; (3) `add_clearance_rule` writes `[class_no; 6]` but Java `DefaultItemClearanceClasses.setAll` loops from i=1 (`rules/DefaultItemClearanceClasses.java:46-49`), leaving the never-read NONE slot 0 at 0 — write `[0, class_no×5]` + fix the comment (behaviorally invisible; `NetClassIr` doc invariant); (4) stale probe citation at `scope/library.rs:1400` (and pre-existing `:1198`) citing nonexistent `/tmp/epic-t4c-images.out` — re-anchor to the Java lines (`Library.java:411-423` `::N`+equalsIgnoreCase, `:250-252` `> 0.001`, `:241` case-sensitive equals) or capture a fresh probe; (5) `board_rules_mut` trait doc (`sink.rs:1197-1209`) describes a write-through the network reader does not (and should not) perform — reword; (6) `network.rs:1648` cites `insertComponents (Network.java:846-852)` — actual `insertComponents` is 832-838, 846-852 is inside `insertLogicalParts` (844-898); (7) `net_classes_mut` return type `&mut Vec<NetClassIr>` → `&mut [NetClassIr]`; drop dead `Clone` derives on private `DsnNetClass`/`DsnClassClass`.

**Part A2 — Task 9 quality-review round (applied 2026-09-14):** two
review-found Java divergences fixed, jar probes `/tmp/t9rev-two.out`
(fixtures `/tmp/t9rev-{eof,closed}.dsn`): (1) the pcb dispatcher
DISCARDS skip_scope's failure — `ScopeKeyword.java:74-76` calls skipScope
bare, so EOF inside an unknown pcb-level scope still folds to Success
(test `t50_eof_in_unknown_pcb_scope_is_discarded`, reader.rs; CASE eof
pins: Success, GEN_MAX 1, WARN_COUNT 0); (2) the closed sub-USER_FIXED
trace drop BURNS an item id — `BasicBoard.java:183-201` constructs the
trace (id allocated in the `Item` ctor, `Item.java:86-89`) before the
guard (test `t51_closed_trace_drop_burns_an_id`, ses_board.rs; CASE
closed pins: open trace stored at id 3, GEN_MAX 3, WARN_COUNT 0).

- [ ] `(wiring ...)`: wires/vias/conduction areas in file order (`Wiring.java` 715 LOC): unknown-layer drop, outside-bbox drop, zero-length skip, duplicate via skip, missing-padstack drop (T30/T31 — each with its digit-normalized warning pinned), `calcFixed` incl. verified T36 behavior, `.N` strip. **Wiring.java's `warnings.add` sites are the ONLY feed of the D12 digest warnings list** (via `ReadScopeParameter.warnings`) — mirrored 1:1 by Rust `state.warnings`; the earlier "wire `push_warning` there" instruction is REFUTED with jar evidence (digest warnings fed solely via `ReadScopeParameter.warnings` — the FRLogger-adjacent sites are log-only, and wiring them would break WARN_COUNT parity), and the dead `push_warning` channel was deleted (spec review 2026-09-14). Also: capture + pin the T37 no-fire fixture (`/tmp/epic-t6-t37-nonet.dsn` pattern — wire veto; left unjar-captured by Task 6) and own the heuristic call site (`DsnReader.java:134-135`, gate `autorouteSettings == null`, not "no `(autoroute ...)` scope").
- [ ] `normalize_all_traces` is NOT called — document at the wiring scope exit (D11 divergence class).
- [ ] `(autoroute_settings ...)` → raw IR (D15, `AutorouteSettings.java` 243 LOC).
- [ ] `(parser ...)` scope: string_quote (default `"`), space_in_quoted_tokens, host_cad/version, constants, reserved cascades (`Parser.java:92-168`).
- [ ] `read_board`: 3-token header hand-scan + NAME state (`DsnReader.java:83-109`), pcb-scope dispatch with skipScope fallback (T24 observable here), post-read heuristic hookup, warnings collection (`:137-146`), `DsnReadResult` enum mirroring `BoardReadResult.java:22-31`.
- [ ] Gates + commit `feat(m1b): wiring + autoroute settings + read_board assembly`.

## Task 10: readMetadata + shared-semantics pin

**Files:** Modify `reader.rs`.

- [ ] `read_metadata` early-stop path (T42, `DsnReader.java:182-280`): parses only parser/resolution/structure then breaks — transliterate the break structure exactly.
- [ ] Shared-semantics pin (D16): test over ALL 175 fixtures asserting `read_metadata` fields == fields from `read_board` result (both Rust-side); plus a one-time jar verification that Java's two paths agree on the same 175 (record in golden capture, Task 11). **D16 jar verification done (Task 12, 2026-09-14, `/tmp/epic-t12-probe.jsh`)**: readMetadata equaled the readBoard-derivable metadata (hostCad, hostVersion, layerCount, unit, resolution, snapAngle via BasicBoard) on all 174 non-ParseError fixtures with result classifications agreeing 175/175 (1 ParseError pair, matching locations); Java's readBoard deliberately returns `Success(board, null, warnings)` so routerSettings has no readBoard-derived counterpart — its fast/full parity is pinned Rust-side (`dsn_metadata_pin.rs`).
- [ ] Gates + commit `feat(m1b): readMetadata fast path with shared-semantics pins`.

## Task 11: Digest oracle + harness commands + goldens committed

**Files:** Create `rust/harness/oracle/DsnParseOracle.java`, `rust/harness/src/dsn_corpus.rs`; modify `rust/harness/src/main.rs` (subcommands `Dsn {Golden, Compare}`), `rust/harness/src/dsn_digest.rs`.

- [ ] Oracle per digest schema (batched JSONL over a manifest file, per-case flush, unknown → exit 3 with id; `BoardStatistics` via Gson, geometry text + SHA-256 in **descending id** order, digit-normalized warnings). Manifest = 175 digest fixtures + 1,157 soak, each line `{id, abs_path}`; generated deterministically by a harness subcommand (`dsn manifest`) from tiers.yaml + directory walks, committed.
- [ ] `dsn golden`: one JVM process (D14), java resolution via existing `oracle.rs`, writes `rust/harness/corpus/dsn-golden.jsonl`; `dsn compare`: java-free, field-for-field diff, ≤20 mismatches printed, exit 1 on any; counts checked (M1a lesson).
- [ ] Regeneration pins (M1a bug-078 class): a test pinning the committed manifest (deterministic manifest generation byte-identical) and the committed golden line count.
- [ ] Capture + commit goldens; record Java-vs-Rust soak parse timing. **Adjudicate D10's CI-set choice here** (full vs `--set digest` if compare >60 s) and write the outcome into this plan (doc-of-record) in the commit.
- [ ] Gates + commit `feat(m1b): DSN parse digest oracle + goldens (175 fixtures + 1157 soak)`.

## Task 12: Parse parity green on all fixtures

**Files:** Modify `epic-dsn/**`, `dsn_corpus.rs` as divergences demand; append traps to the T-table if new ones surface (doc-of-record, in the fix commit — M1a Task 10 precedent).

- [ ] `dsn compare` → iterate to 175/175 + soak green. Classify EVERY mismatch against T24–T46 BEFORE fixing. Anti-fraud: never weaken the comparator; never skip a fixture Java parses; Java-throwing inputs follow the documented `result:"ParseError"` equivalence. **Outcome (2026-09-14, Task 12)**: census before the fix = digest 105/175, soak 675/1157; the single fix class was **T49** (lazy placement transform on image keepouts — see the new T49 row) and it flipped the gate to `compare --set all: 1332 fixture(s), 0 mismatch(es), 1 ledgered divergence(s)` in 2.6 s. The one ledger entry is dsn-0151 (normalizeAllTraces, D11/M2 class, allowed_fields stats+geometry_sha256) with stale-entry + outside-allowance enforcement pinned in `dsn_corpus.rs`.
- [ ] Algebraic/robustness invariants: re-parse determinism (parse twice → identical digest on all 175), idempotent skip_scope, lexer never panics on the 39 root .ses files (not DSN — must produce ParseError, not a panic).
- [ ] Gates + commit `test(m1b): DSN parse parity green — 175 fixtures + soak vs Java oracle`.

**Task 11 quality-review census leads:**
1. **Circle-keepout centers dominate**: 550 of 552 pre-parity mismatches involve `shape:circle` keepouts where Java applies a component/coordinate translation to the circle center (e.g. dsn-0002 `K 362 ... circle 1370841 -1036066 10000` vs Rust `39500 0 10000`) — likely a single transform-site fix flips nearly the whole gate. Fixing this is the FIRST Task-12 move.
2. **dsn-0151 `Issue723-CombineStackOverflow.dsn`**: Java `normalizeAllTraces()` combines 4,000 wire segments into 1 trace (items=2/traces=1) vs Rust 4,001/4,000 — the documented normalizeAllTraces M2 divergence class (this plan, D11). Task 12 must decide how the gate handles it (explicit documented exclusion or comparator divergence-marker — NOT a silent skip; the anti-fraud rule above stands).
3. **Keepout-kind blind spot**: the digest `K` line erases the keepout/via_keepout/place_keepout distinction (see `dsn_digest.rs` module doc) — a kind-swap regression is digest-invisible; decide whether Task 12/M2 extends the format on BOTH sides.

## Task 13: SES emission + canonical compare + byte-diff classification

**Files:** Create `src/write_scope.rs`, `src/ses/writer.rs`; harness: `rust/harness/src/ses_compare.rs` + oracle extension (or `SesEmitOracle.java` — parse→emit per fixture).

- [ ] `write_scope.rs`: `IndentFileWriter` (UTF-8, LF, 2-space — `IndentFileWriter.java:12,19,55`), `IdentifierType` quoting incl. the de-quote off-by-one (T38) and reserved sets; pin quote-decisions on names with spaces/apostrophes/leading digits/non-ASCII.
- [ ] `ses/writer.rs` per `SesWriter.java`: session scaffold (`:73-94`), placement grouped in LIBRARY order (packages table order, ids ascending within — grouping loop `:106-111`, writePadstack `:271-307`; NOT placement-scope order — Task 4 fix-round correction), library_out first-name-wins dedup (`:243-252`), network_out nets 1..max with items in DESCENDING id (T39), coordinates via M1a `java_round` (T41), rotation 3-decimal trim (`:259-269`), fixed states → `(type fix)`/`(type protect)` mapping (`:478-490`), scaleFactor re-derivation (T43, `:80-82`). NO endpoint snapping (T40) — assert instead that emitted endpoints equal parsed endpoints.
- [ ] `ses_compare.rs`: tiny sexpr parser (harness-only), token-tree equality, numbers exact; `dsn ses-golden` captures Java's parse→emit .ses per tier fixture (committed as `rust/harness/corpus/ses/<fixture>.ses.golden`); `dsn ses-compare` runs Rust emit + canonical compare.
- [ ] Tier A+B canonical compare green; byte-diff report: per fixture, byte-equal yes/no + diff class (T40-snap / normalization / genuine). Exit criterion: canonical green, zero unclassified byte diffs; genuine diffs are bugs → fix before exit.
- [ ] Gates + commit `feat(m1b): SES writer + canonical parity on tiers A/B + byte-diff report`.

## Task 14: M1b exit criteria + docs

- [ ] **Step 1:** `cd rust && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check` — green.
- [ ] **Step 2:** `EPIC_SKIP_GRADLE=1 cargo run -q -p epic-harness -- dsn compare` (+ the D10-adjudicated set) and `... -- dsn ses-compare` — green, java-free; add both to `rust-check.yml` after `corpus compare`.
- [ ] **Step 3:** Update `rust/README.md` (M1b done, dsn commands), epic-dsn lib.rs milestone note, `.wolf/memory.md` M1b row (M0/M1a precedent), and fix the stale fixture count in the design doc §6 M1 row (162 → the enumerated 175+1157 reality, with a note).
- [ ] **Step 4:** Commit `docs(m1b): README + CI steps + design-doc fixture count + session log`. Report M1b exit = digest green + SES canonical green + byte-diffs classified + gates green. M2 planning follows.

---

## Self-review notes

1. **Spec coverage vs design §6 M1 row:** "DSN parses all fixtures" = Tasks 11–12 (digest set per D10, superseding the stale 162); "SES round-trip parity on Tier A" = Task 13 (canonical compare per D11 — byte-parity explicitly re-based to M2, recorded, not silently dropped). §5 parity gates: digest + ses-compare both CI-wired java-free.
2. **No placeholders:** novel code (lexer semantics, sink trait, digest schema, oracle, comparator) has concrete specs + pinned values; bulk scope readers reference exact Java files/lines with per-scope trap lists — the Java source is the normative spec (M1a precedent: transliteration cannot be both complete and inlined).
3. **Type consistency:** `DsnReadResult` mirrors `BoardReadResult`; `BoardSink` methods are introduced once (Task 4) and consumed by Tasks 5–9; digest canonical-line formats defined once (Task 4) and used by oracle (Task 11) and SES compare (Task 13). Trap numbering continues M1a's T1–T23 as T24–T46 (recon's T35b renumbered to T36, subsequent traps shifted +1).
4. **Known honest simplifications:** no normalization/no snapping in SES emission (D11, tracked); warnings digit-normalized (D12); soak digests may be CI-opt-in pending the Task 11 timing measurement (D10).
