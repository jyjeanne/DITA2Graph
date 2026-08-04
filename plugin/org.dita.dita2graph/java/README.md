# plugin/org.dita.dita2graph/java

Builds `../lib/dita2graph-core.jar` -- the Java side of the DITA-OT
plugin (`org.dita.dita2graph.tasks.ExtractTask`, docs/plugin-specification.md
§2.1/§3.1). A standalone Gradle project so it builds independently of
`gradle-build/` (which only *consumes* the jar this produces) and of the
Rust workspace at the repo root.

## Building and testing

```bash
./gradlew test    # runs DitaModelExtractorTest against a fixture shaped
                   # like real DITA-OT 4.4 resolved output
./gradlew jar      # writes ../lib/dita2graph-core.jar
```

Requires `dita2graph-core` (the Rust binary, `../../../core/dita2graph-core`)
on `PATH` or pointed at via `DITA2GRAPH_CORE_BIN` for the jar to actually
do anything once installed in DITA-OT -- the test suite here doesn't need
it (`DitaModelExtractor`, the part under test, only builds the normalized
model; it doesn't invoke the Rust binary, `ExtractTask` does that).

Compiled at `--release 21`: this sandbox only has JDK 21, not the spec's
Java 25 floor (§1.1) -- see `build.gradle.kts` for why, and
`docs/dev/phase-0-findings.md` for the broader JDK-25-unavailable
constraint already documented in `gradle-build/README.md`.

## What `ExtractTask` actually derives from DITA markup

Deliberately limited to what's directly derivable without heuristic
inference (see the class docs in `DitaModelExtractor.java` for the exact
rules): a map's direct `<topicref>` children become `contains` edges;
`<xref>`/`<link>` elements become `requires` if they carry a `keyref`,
`references` otherwise. `applies-to`, `related-to`, and `generated-from`
all need inference this extractor doesn't attempt (§3.3, §13 future
work) and never appear in its output.
