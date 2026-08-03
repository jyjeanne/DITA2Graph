# gradle-build

The live integration harness for `docs/plugin-specification.md` §8.2's
Kotlin DSL example, pointed at this repo's own `../sample-docs/` and
`../plugin/org.dita.dita2graph`. Not a template for downstream consumers
(who would put `build.gradle.kts` in their own DITA content repo) — this
is what closed Phase 0 finding 4 (`../docs/dev/phase-0-findings.md`):
does a live DITA-OT run actually resolve `sample-docs/` and dispatch to
our transtype the way the spec assumes?

## Requirements

Gradle 9.0+ (wrapper pinned to 9.6.1, run via `./gradlew`), a JDK on
`PATH` or `JAVA_HOME` (the spec targets Java 25, §1.1; this harness has
been run against JDK 21 in a sandbox without JDK 25 available — DITA-OT
4.4 itself only requires 17+, so this doesn't affect the DITA-OT-side
findings, only means JDK 25-specific behavior is unverified).

## Running it

```bash
./gradlew validateDocs checkLinks   # real, passes today
./gradlew installDita2Graph          # fails: lib/dita2graph-core.jar doesn't exist yet (§2.1)
./gradlew buildKnowledgeGraph        # fails via the same missing-jar dependency
```

`build/` and `.gradle/` are gitignored — `build/dita-ot/dita-ot-4.4/` in
particular is an ~80MB downloaded DITA-OT install, not something to
commit. Re-running `./gradlew downloadDitaOt` re-fetches it.

See `docs/dev/phase-0-findings.md` finding 5 for the full story: what
failed, why, and the plugin.xml/build.xml fixes that got the
`dita2graph` transtype dispatching correctly through a real DITA-OT 4.4
preprocessing run.
