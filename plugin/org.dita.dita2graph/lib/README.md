# lib/dita2graph-core.jar

Built by the Gradle project at `../java/` -- run `./gradlew jar` there
(or `./gradlew build` to test-and-build) and it lands here. Not committed
to the repo: it's a build artifact, gitignored, and reproducible from
`../java/src/`.

Contains `org.dita.dita2graph.tasks.ExtractTask` (the `dita2graph:extract`
Ant task referenced from `../build.xml`) plus its helper classes and an
`antlib.xml` binding. `ExtractTask` reads DITA-OT's resolved `.job.xml`
and temp-directory topics/maps, builds the normalized DITA model (§3.2),
and shells out to the `dita2graph-core` Rust binary (`../../../core/dita2graph-core`,
found via the `DITA2GRAPH_CORE_BIN` env var or `PATH`) to write the OKF
bundle. Verified end-to-end against a live DITA-OT 4.4 run --
`docs/dev/phase-0-findings.md` findings 6 and 7 cover what that took to
get right.

Nothing in `core/` or `mcp/` depends on this jar existing — both build and
test standalone via `cargo build`/`cargo test` from the repo root. This
jar is only needed to wire the Java-side DITA-OT hook (§2.2) up to that
already-working Rust pipeline.

Compiled at `--release 21` today, not the spec's Java 25 floor (§1.1):
this sandbox only has JDK 21 available (see `../java/build.gradle.kts`
for why). `.java-version` at the repo root still documents the real
target; bump the Gradle build once a JDK 25 is available to compile and
test against.
