# lib/dita2graph-core.jar

Built by the Gradle project at `../java/` -- run `./gradlew jar` there
(or `./gradlew build` to test-and-build) and it lands here. Not committed
to the repo: it's a build artifact, gitignored, and reproducible from
`../java/src/`.

Contains `io.github.jyjeanne.dita2graph.tasks.ExtractTask` (the `dita2graph:extract`
Ant task referenced from `../build.xml`) plus its helper classes and an
`antlib.xml` binding. `ExtractTask` reads DITA-OT's resolved `.job.xml`
and temp-directory topics/maps, builds the normalized DITA model (§3.2),
and shells out to the `dita2graph-core` Rust binary (`../../../core/dita2graph-core`)
to write the OKF bundle: the one the `DITA2GRAPH_CORE_BIN` env var names,
else the build the release ZIP ships for the platform in
`../bin/<os>-<arch>/` (`linux-x86_64`, `linux-aarch64`, `macos-x86_64`,
`macos-aarch64`, `windows-x86_64`), made executable first since `dita
install` drops the Unix permission bits, else the one on `PATH`. Verified end-to-end against a live DITA-OT 4.4 run --
`docs/dev/phase-0-findings.md` findings 6 and 7 cover what that took to
get right.

Nothing in `core/` or `mcp/` depends on this jar existing — both build and
test standalone via `cargo build`/`cargo test` from the repo root. This
jar is only needed to wire the Java-side DITA-OT hook (§2.2) up to that
already-working Rust pipeline.

Compiled at `--release 17`: the jar runs inside DITA-OT's own JVM, and
DITA-OT 4.x runs on Java 17 (`../java/build.gradle.kts`).
