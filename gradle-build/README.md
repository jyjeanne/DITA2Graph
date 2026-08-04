# gradle-build

The live integration harness for `docs/plugin-specification.md` §8.2's
Kotlin DSL example, pointed at this repo's own `../sample-docs/` and
`../plugin/org.dita.dita2graph`. Not a template for downstream consumers
(who would put `build.gradle.kts` in their own DITA content repo) — this
is what closed Phase 0 finding 4 (`../docs/dev/phase-0-findings.md`):
does a live DITA-OT run actually resolve `sample-docs/`, dispatch to our
transtype, and produce a real bundle the way the spec assumes?

## Requirements

Gradle 9.0+ (wrapper pinned to 9.6.1, run via `./gradlew`), a JDK on
`PATH` or `JAVA_HOME` (the spec targets Java 25, §1.1; this harness has
been run against JDK 21 in a sandbox without JDK 25 available — DITA-OT
4.4 itself only requires 17+, so this doesn't affect the DITA-OT-side
findings, only means JDK 25-specific behavior is unverified), and
`lib/dita2graph-core.jar` built (`../plugin/org.dita.dita2graph/java`,
`./gradlew jar`) plus the `dita2graph-core` Rust binary on `PATH` or
pointed at via `DITA2GRAPH_CORE_BIN` for `buildKnowledgeGraph` to
actually produce output.

## Running it

```bash
export DITA2GRAPH_CORE_BIN=/path/to/dita2graph-core   # or put it on PATH

./gradlew validateDocs checkLinks   # real, passes
./gradlew installDita2Graph          # real, installs the plugin (zips it first, see below)
./gradlew buildKnowledgeGraph        # real, produces build/dita2graph/{okf,rag}/ (unfiltered)

./gradlew buildKnowledgeGraphPublic    # real, filtered with ../sample-docs/public.ditaval
./gradlew buildKnowledgeGraphInternal  # real, filtered with ../sample-docs/internal.ditaval

./gradlew validateBrokenDoc          # expected to FAIL -- see below
```

## Public/internal DITAVAL split (§6.1)

`buildKnowledgeGraphPublic` and `buildKnowledgeGraphInternal` build the
same `../sample-docs/user-guide.ditamap` with two different DITAVAL
profiles into separate output directories
(`build/dita2graph-public/`, `build/dita2graph-internal/`). The map's
`topics/internal-notes.dita` topicref carries `audience="internal"`;
`public.ditaval` excludes it, `internal.ditaval` includes it. Confirmed
directly, not just asserted:

```bash
diff <(ls build/dita2graph-public/okf/topics) <(ls build/dita2graph-internal/okf/topics)
# > internal-notes.md   (present only on the internal side)
```

This is §6.1's point made concrete: the filtering happens during
DITA-OT preprocessing, before `dita2graph`'s extraction step ever runs
on the excluded topic — a topic that was never extracted cannot leak
through the MCP server, regardless of what the server-side query logic
does or doesn't check. The same filtered normalized model feeds both
outputs (§13.1), so `rag/chunks.jsonl` agrees with `okf/topics/` —
`internal-notes` is absent from `dita2graph-public/rag/chunks.jsonl`
the same way it's absent from `dita2graph-public/okf/topics/`, not just
coincidentally similar.

## `rag/` output (§13.1)

`buildKnowledgeGraph` (and the public/internal variants above) also
write `build/dita2graph/rag/chunks.jsonl` and `rag/metadata.json` —
the first implemented piece of §13.1's "single pass, two correlated
outputs" design direction: one enriched, plain-text record per topic
(title, combined shortdesc+body text, tags, and an `okfNode` field that
joins it back to the matching `okf/topics/*.md` concept), derived from
the same in-memory normalized model `okf/` is written from, not a
second parse of the DITA source. Nothing consumes this file yet — no
search, no embeddings, no MCP tool — it exists so that layer has real
data to work with when it's built. `dita2graph-core`'s secret scan
(§6.4) covers `rag/` the same as `okf/`, run as a separate pass since
`rag/`'s JSONL isn't OKF-conformant format so `okf-validator` doesn't
apply to it.

`build/` and `.gradle/` are gitignored — `build/dita-ot/dita-ot-4.4/` in
particular is an ~80MB downloaded DITA-OT install, not something to
commit. Re-running `./gradlew downloadDitaOt` re-fetches it.

## `validateBrokenDoc`: proving the validation gate actually works

Deliberately **not** wired into `buildKnowledgeGraph`'s dependency
chain — it's expected to fail, and Gradle would fail the whole build if
a normal dependency did. It validates `../sample-docs-invalid/` (an
`<xref href="does-not-exist.dita"/>` to a file that isn't there), which
is the Phase 4 exit criterion (`docs/plugin-specification.md` §12) made
concrete: proof, not just a claim, that DITA-OT's own validation rejects
broken source before `dita2graph` ever runs. Run it and check the exit
code:

```bash
./gradlew validateBrokenDoc; echo "exit: $?"   # expect a non-zero exit
```

See `../sample-docs-invalid/README.md` for why this fixture uses a
broken `href` rather than a broken `keyref` — the two behave differently
(an unresolvable `keyref` is only informational and does *not* fail
DITA-OT's build, confirmed directly rather than assumed).

## Local-install detail: plugins need a ZIP

`installDita2Graph` zips `../plugin/org.dita.dita2graph` via the
`zipDita2GraphPlugin` task before installing it —
`DitaOtInstallPluginTask`'s local-install path needs a path to a `.zip`,
not a plugin source directory (confirmed the hard way: pointing it at
the raw directory fails with the same error `java.util.zip` gives
reading a directory as a zip stream).

See `docs/dev/phase-0-findings.md` findings 5–7 for the full story: what
failed, why, and the `plugin.xml`/`build.xml`/`build.gradle.kts` fixes
that got everything here working end to end against a real DITA-OT 4.4.
