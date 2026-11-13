# plugin/io.github.jyjeanne.dita2graph/java

Builds `../lib/dita2graph-core.jar` -- the Java side of the DITA-OT
plugin (`io.github.jyjeanne.dita2graph.tasks.ExtractTask`, docs/plugin-specification.md
§2.1/§3.1). A standalone Gradle project so it builds independently of
`gradle-build/` (which only *consumes* the jar this produces) and of the
Rust workspace at the repo root.

## Building and testing

```bash
./gradlew test    # runs DitaModelExtractorTest against a fixture shaped
                   # like real DITA-OT 4.4 resolved output
./gradlew jar      # writes ../lib/dita2graph-core.jar
```

Once installed in DITA-OT, the jar runs the `dita2graph-core` Rust
binary (`../../../core/dita2graph-core`): the one `DITA2GRAPH_CORE_BIN`
names, else the build the plugin ships for the platform in
`../bin/<os>-<arch>/`, else the one on `PATH` (`CoreBinary.java`). The
test suite here doesn't need it (`DitaModelExtractor`, the main part
under test, only builds the normalized model; `ExtractTask` runs the
binary).

Compiled at `--release 17`, whatever JDK 17 or later builds it: the jar
runs inside DITA-OT's own JVM, and DITA-OT 4.x runs on Java 17 (see
`build.gradle.kts`).

## What `ExtractTask` actually derives from DITA markup

Deliberately limited to what's directly derivable without heuristic
inference (see the class docs in `DitaModelExtractor.java` for the exact
rules): a map's direct `<topicref>` children become `contains` edges;
`<xref>`/`<link>` elements become `requires` if they carry a `keyref`,
`references` otherwise. `applies-to`, `related-to`, and `generated-from`
all need inference this extractor doesn't attempt (§3.3, §13 future
work) and never appear in its output.
