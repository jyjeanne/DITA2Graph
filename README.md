# DITA2Graph

A DITA-OT plugin that converts DITA content into a semantic knowledge
graph (using [OKF](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
v0.2 as the representation) and exposes it to AI agents over MCP.

Full design, rationale, and roadmap: **[`docs/plugin-specification.md`](docs/plugin-specification.md)**.
Implementation status against that roadmap's Phase 0–2: **[`docs/dev/phase-0-findings.md`](docs/dev/phase-0-findings.md)**
and the per-phase "Status" notes in the spec's §12.

## Status

The core pipeline is real and runs end to end: DITA-OT preprocessing →
Java extraction → Rust OKF writer → validated bundle → MCP server.

| Component | Status |
|---|---|
| `docs/plugin-specification.md` | Design spec, complete |
| `core/dita2graph-core` (Rust) | Working: normalized-model types, OKF bundle writer, RAG content index writer (`rag/`, §13.1), `build`/`validate`/`query` CLI, passing tests |
| `mcp/dita2graph-mcp` (Rust) | Working: JSON-RPC-over-stdio MCP server with the full §5.2 tool set, passing tests; takes a bundle root directly or via `--config <mcp-server.toml>` (written by `dita2graph-core build --mcp true`, §5.4). §5.1's Resources (`resources/list`/`resources/read`) are not implemented |
| `plugin/org.dita.dita2graph/java` (Java) | Working: `ExtractTask` parses DITA-OT's resolved output into the normalized model, shells out to `dita2graph-core`; unit tested. Walks nested `topicref`/`topichead`/`topicgroup` map structures at any depth, not just the top level, excludes DITA-OT's auto-generated `related-links` navigation from cross-reference extraction, and enforces `args.dita2graph.depth` to limit how many containment levels are captured (`docs/dev/phase-0-findings.md` finding 11) |
| `plugin/org.dita.dita2graph` (Ant/XML) | **Verified end-to-end** against a live DITA-OT 4.4: installs, dispatches, produces a real `okf_validator`-passing bundle, and accepts `--args.dita2graph.*` CLI overrides (all five, via `plugin.xml`'s `<param>` declarations — previously silently rejected by DITA-OT's own CLI parser, see `docs/dev/phase-0-findings.md` finding 10) |
| `gradle-build/` | Real Gradle 9.6.1 + Kotlin DSL project; `./gradlew buildKnowledgeGraph` runs the entire pipeline for real, plus `buildKnowledgeGraphPublic`/`buildKnowledgeGraphInternal` for the DITAVAL split (§6.1) |
| `sample-docs/` | A small fixture DITA project, confirmed to resolve correctly and extract correctly through the full pipeline, including one `audience="internal"` topic used to prove the DITAVAL split actually filters |
| `sample-docs-nested/` | A fixture exercising nested `topicref`/`topichead`/`topicgroup` and the `related-links` exclusion (finding 11) |
| CI | Real: `rust.yml`/`java.yml` unit-test each side, `integration.yml` runs the full pipeline (including the DITAVAL split, the nested-map fixture, and the broken-input negative test) against a live DITA-OT 4.4 |
| Security (§6) | Secret-leakage detection shipped (`core/dita2graph-core/src/secrets.rs`, build-breaking, §6.4, covers `okf/` and `rag/`); public/internal DITAVAL split demonstrated (§6.1); HTTP transport auth (§6.3) not yet implemented — stdio only |
| Licensing | Decided and shipped: dual **MIT OR Apache-2.0** across the whole repo (`LICENSE`, `NOTICE`) |
| Hybrid graph+RAG architecture (§13.1) | Nearly done: body-text extraction, `rag/chunks.jsonl` + `rag/metadata.json` (same single pass as `okf/`), `search_content` (graph-narrowed, keyword-frequency-ranked content search), and `analyze_impact` (reverse, transitive graph traversal with a text excerpt per affected concept). Still design-only: node-level embeddings (the heavier, not-yet-committed direction) |

See `docs/dev/phase-0-findings.md` for what's still narrower than the
full spec envisions (`applies-to`/`related-to`/`generated-from`
relation inference, `navref`/`anchorref`/`mapref` map composition, and
§5.1's MCP Resources — `dita://topics` etc. — which `dita2graph-mcp`
doesn't implement at all, only the §5.2 tools) — real gaps, documented
rather than hidden. All five `args.dita2graph.*` parameters, including
`mcp`, are now functionally wired end to end (finding 12).

## Toolchain requirements

Per `docs/plugin-specification.md` §1.1: **Gradle 9.0 minimum**, **Java 25
(latest LTS)**, **Rust latest stable** (currently 1.97.1, pinned in
`rust-toolchain.toml` — `rustup` picks it up automatically). `.java-version`
at the repo root pins the Java requirement for tooling that reads it.
`plugin/org.dita.dita2graph/java` currently compiles at `--release 21`
(no JDK 25 available where this was built/tested — see that
subproject's README).

## Quickstart (what works today)

```bash
# Build the Rust workspace (rustup fetches the pinned toolchain automatically)
cargo build --release --workspace

# Build the DITA-OT plugin's Java side
(cd plugin/org.dita.dita2graph/java && ./gradlew jar)

# Install the plugin and run it against the sample project, via the real
# Gradle/Kotlin DSL harness. First run downloads DITA-OT 4.4 (~50MB
# compressed, ~80MB installed) into gradle-build/build/dita-ot/ -- give
# it a minute the first time; later runs reuse the cached install.
export DITA2GRAPH_CORE_BIN="$PWD/target/release/dita2graph-core"
(cd gradle-build && ./gradlew buildKnowledgeGraph)

# Inspect the real, generated, validated bundle
cat gradle-build/build/dita2graph/okf/topics/installing-product.md
./target/release/dita2graph-core validate --bundle gradle-build/build/dita2graph/okf

# Inspect the RAG content index written alongside it, from the same
# extraction pass (§13.1) -- search_content/analyze_impact below read this
cat gradle-build/build/dita2graph/rag/chunks.jsonl

# Talk to the MCP server directly over stdio (one JSON-RPC message per line)
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_topics","arguments":{"query":"install"}}}' \
  | ./target/release/dita2graph-mcp gradle-build/build/dita2graph

# Content search (searches rag/'s actual text, ranked by keyword frequency --
# not just titles/ids the way search_topics is)
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_content","arguments":{"query":"install product"}}}' \
  | ./target/release/dita2graph-mcp gradle-build/build/dita2graph

# Impact analysis: what depends on this topic, transitively, with a text
# excerpt of each affected concept
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"analyze_impact","arguments":{"topicId":"configuration"}}}' \
  | ./target/release/dita2graph-mcp gradle-build/build/dita2graph
```

To register the server with Claude Code once you have a bundle built:

```bash
claude mcp add dita2graph -- ./target/release/dita2graph-mcp gradle-build/build/dita2graph
```

### Available MCP tools

Once registered, an agent can call:

| Tool | What it does |
|---|---|
| `search_topics(query)` | Plain text match against topic/map titles and ids |
| `search_content(query, topicId?, relation?, depth?)` | Ranked full-text search over `rag/` content; scope it to a topic's graph neighborhood for hybrid graph+content queries (§13.1) |
| `find_related_topics(topicId, relation?)` | Direct relations from a topic |
| `explain_task(topicId)` | Title, description, and key relations for a topic |
| `trace_dependencies(topicId, depth?)` | Forward `requires` chain from a topic |
| `analyze_impact(topicId, depth?)` | Reverse, transitive traversal — everything that would be affected by changing this topic, with content excerpts (§13.1) |
| `generate_summary(id)` | Title + description for a topic or map |
| `validate_bundle()` | Re-runs `okf-validator` + the secret-leak scan on demand |

Full argument shapes and behavior: `docs/plugin-specification.md` §5.2.

### Using your own DITA project

The `gradle-build/` harness above is a demo pointed at this repo's own
`sample-docs/`, not a template to copy into your own project. To run
against your own DITA content instead, install the plugin into your own
DITA-OT and invoke it directly — see `docs/plugin-specification.md` §15
(Appendix A: Quickstart) for the full sequence (`dita --install`, then
`dita --format dita2graph`).

## Repository layout

```
docs/plugin-specification.md    # design spec, source of truth
docs/dev/phase-0-findings.md    # spike results and decisions made from them
core/dita2graph-core/           # Rust: normalized model, OKF writer, CLI (§3)
mcp/dita2graph-mcp/             # Rust: MCP server (§5)
plugin/org.dita.dita2graph/     # DITA-OT plugin: plugin.xml/build.xml/cfg (§2)
plugin/org.dita.dita2graph/java # Java: ExtractTask, builds lib/dita2graph-core.jar
gradle-build/                   # Live Gradle/Kotlin DSL integration harness (§8)
sample-docs/                    # fixture DITA project used by tests/demos
```
