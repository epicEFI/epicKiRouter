# t2agent — the buglog-189 live-process recording instrument (M5-T2, lever (a))

A `premain` javaagent that lands the M4-T12 recording form (`completeShape`
input/result logging, the ported `instrument_T12RecordingTree45` subclass
semantics) INSIDE the real CLI process, on the tree class the CLI actually
constructs.

## Why an in-place class transform, not a subclass/factory swap

`SearchTreeManager` constructs the autoroute tree with a hardcoded
`new ShapeSearchTree45Degree(this.board, clearanceClassIndex)`
(SearchTreeManager.java:154; data-driven only by the board's angle
restriction), the class is chosen in an if/else chain with no factory/setter,
and the base `ShapeSearchTree` constructor is package-private — an
out-of-package recording subclass cannot even be constructed. The agent
therefore instruments `completeShape` IN PLACE on `ShapeSearchTree45Degree`
(+ the `ShapeSearchTree` base) via ByteBuddy Advice inlined at method
entry/exit — the same method the T12 subclass would have hooked, on the same
live instances.

## Build (JDK 25; ByteBuddy from the local Gradle cache; frozen jar UNTOUCHED)

The cache directory hash IS the artifact's canonical Maven Central sha1
(`3856bfab61beb23e099a0d6629f2ba8de4b98ace` for byte-buddy-1.17.7.jar, verified
against repo1.maven.org's published `.sha1`), so the pin is machine-independent;
on a cache miss, fetch
`https://repo1.maven.org/maven2/net/bytebuddy/byte-buddy/1.17.7/byte-buddy-1.17.7.jar`,
verify its sha1, and use that path in BB below.

Run from this directory (`cd rust/harness/oracle/t2agent`):

```sh
BB=$HOME/.gradle/caches/modules-2/files-2.1/net.bytebuddy/byte-buddy/1.17.7/3856bfab61beb23e099a0d6629f2ba8de4b98ace/byte-buddy-1.17.7.jar
JAR=build/libs/freerouting-current-executable.jar   # never rebuilt/modified
JDK=$HOME/.jdks/jdk-25.0.4.1+1
$JDK/bin/javac -cp "$BB:$JAR" -d classes src/t2agent/*.java
rm -rf fatroot && mkdir fatroot && (cd fatroot && $JDK/bin/jar xf $BB)
$JDK/bin/jar cfm t2agent.jar manifest.txt -C classes . -C fatroot .
```

## Run (bm06 fanout-only face; the T12 invocation shape)

Run from the repo root (the `-de`/`-do`/log paths below are repo-relative):

```sh
cd /path/to/epicrouter   # the repo root
timeout 300 $JDK/bin/java -Xmx4g \
  -javaagent:'.../t2agent.jar=log=/abs/path/t2.log;net=21' \
  -jar $JAR -de scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm06.dsn \
  -do /abs/path/out.ses --router.result_json=/abs/path/manifest.json \
  --usage_and_diagnostic_data.disable_analytics=true --gui.enabled=false \
  --api_server.enabled=false --router.autorouter.enabled=false \
  --router.fanout.enabled=true
```

Agent options (semicolon-separated, `-javaagent:t2agent.jar=k=v;k=v`; the ONE
normative surface is the options block in T2Log.java — edit the two together):
`log=<path>` (sink; default `logs/M5-T2/scratch/agent/t2agent.log`),
`net=<n>` (row filter; default 21, the bm06 fork net), `maxrows=<n>` (row
budget; default 2000000), `bandhi=<int>` (dump-trigger upper band edge,
exclusive; default -911000), `bandlo=<int>` (lower edge, exclusive; default
-911700), `bandlayer=<int>` (dump-trigger layer; default 0), `dumpcap=<n>`
(max band dumps; default 32), `margin=<int>` (dump band margin in DBU; default
2000). The DEFAULTS reproduce the M5-T2 bm06 capture byte-for-byte — any T3
retargeting must pass overrides, never edit defaults.

Transparency discipline (M5-T2 evidence): compare the run's stdout board
hashes against the uninstrumented face (`b84a773a...` at fanout start,
per-pass hashes) and byte-compare out.ses across runs — an instrumented run
that moves any hash or SES byte is a perturbing instrument, not evidence.

## Counterface use (no -javaagent)

The fat jar doubles as a plain classpath helper for jshell band dumps:
`t2agent.T2Log.init("log=/abs/path")` + `t2agent.T2Dump.dump(tree, octagon)`.
The worked example (`logs/M5-T2/evidence/instrument_counterface.jsh`) is
git-ignored — it exists in the M5-T2 evidence record, not on a fresh clone;
the two calls above are the complete contract.
