# Tutorial: install DITA2Graph and query a doc set over MCP

A complete, hands-on walkthrough: install the plugin, build a knowledge
graph, and ask questions about it through MCP — first against the
bundle project's own sample docs (fastest way to see it work), then
against your own existing DITA project.

This tutorial only uses commands and behavior that are actually shipped
today (see the [Status table](../README.md#status) for what that
covers). Where something is still a roadmap item, it's called out
explicitly rather than glossed over.

## Prerequisites

- **DITA-OT 4.4** reachable on your machine (the Gradle harness in Part
  1 downloads its own copy automatically; installing on your own project
  in Part 2 assumes you already have one).
- **Rust** (latest stable — `rustup` picks up the pinned version from
  `rust-toolchain.toml` automatically) to build `dita2graph-core` and
  `dita2graph-mcp`.
- **Java 25** (or 21 — see `plugin/org.dita.dita2graph/java/README.md`)
  and **Gradle 9.0+** to build the plugin's Java extractor.
- [Claude Code](https://claude.ai/code) (or another MCP-capable client)
  if you want to ask questions through an agent rather than raw
  JSON-RPC.

Build the two Rust binaries once, up front — both parts of this
tutorial use them:

```bash
cargo build --release --workspace
# binaries land in target/release/dita2graph-core and
# target/release/dita2graph-mcp
```

## Part 1 — five-minute walkthrough on the bundled sample project

This uses `gradle-build/`, this repo's own live integration harness
pointed at `sample-docs/` — the fastest way to see the whole pipeline
run against a real DITA-OT install without touching your own content
first.

### 1.1 Build and install the plugin, then run the pipeline

```bash
cd gradle-build
./gradlew buildKnowledgeGraph
```

First run downloads DITA-OT 4.4 (~50MB compressed) into
`gradle-build/build/dita-ot/` and caches it for later runs. This task
zips `plugin/org.dita.dita2graph`, installs it into that DITA-OT copy,
validates `sample-docs/user-guide.ditamap`, checks its links, then runs
the `dita2graph` transtype end to end.

### 1.2 Inspect the generated bundle

```bash
ls build/dita2graph/okf/topics/
cat build/dita2graph/okf/topics/installing-product.md
```

`sample-docs/` is a small fixture with four topics: `configuration`
(a concept), `installing-product` (a task that requires
`configuration`), `installing-product-prereqs`, and `internal-notes`
(marked `audience="internal"`, relevant in 1.5 below). Each becomes one
OKF concept document — plain markdown with a YAML frontmatter block,
readable without any tooling at all.

### 1.3 Validate explicitly

```bash
../target/release/dita2graph-core validate --bundle build/dita2graph/okf
```

Re-runs `okf-validator` (schema conformance, every relation/link target
actually resolving) plus the build-breaking secret-leak scan. The
`buildKnowledgeGraph` task above already gates on this, but it's useful
standalone — e.g. after hand-editing a bundle, or in a CI step of your
own.

### 1.4 Talk to the MCP server directly

The Gradle task didn't pass `--args.dita2graph.mcp=true`, so there's no
`mcp/mcp-server.toml` yet — point `dita2graph-mcp` at the bundle root
directly instead (it takes a bare positional path as a fallback, see
`mcp/dita2graph-mcp/src/main.rs`):

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search_topics","arguments":{"query":"install"}}}' \
  | ../target/release/dita2graph-mcp build/dita2graph
```

You should get back a JSON-RPC response whose `result` text includes
`Installing Product (task) [installing-product]`.

### 1.5 Register it with Claude Code and ask real questions

```bash
claude mcp add dita2graph -- ../target/release/dita2graph-mcp build/dita2graph
```

Now, in a Claude Code session, ask documentation questions in plain
English. Given this fixture's actual content, these are grounded,
verifiable examples — not hypothetical:

| You ask | What should happen |
|---|---|
| "What topics mention installing?" | Calls `search_topics`, returns `installing-product` (and `installing-product-prereqs`) |
| "What does installing the product require?" | Calls `find_related_topics` or `explain_task` on `installing-product`, returns `requires -> Configuration Overview (configuration)` |
| "What's the full dependency chain for installing-product?" | Calls `trace_dependencies(topicId: "installing-product")`, walks the forward `requires` chain |
| "If I change the Configuration Overview topic, what's affected?" | Calls `analyze_impact(topicId: "configuration")` — a reverse, transitive traversal that should surface `installing-product` as a dependent, with a text excerpt |
| "Summarize the configuration topic" | Calls `generate_summary` or `explain_task` on `configuration` |

The signal that the integration is actually working end to end: Claude
Code routes these through the typed MCP tools above, not a raw file
read of `sample-docs/` or the generated markdown (§5.3 of
`docs/plugin-specification.md`).

### 1.6 See the audience/product split in action

`sample-docs/user-guide.ditamap` tags `internal-notes` with
`audience="internal"`. Build both DITAVAL-filtered variants and diff
them:

```bash
./gradlew buildKnowledgeGraphPublic buildKnowledgeGraphInternal
diff <(ls build/dita2graph-public/okf/topics/) <(ls build/dita2graph-internal/okf/topics/)
```

`internal-notes.md` exists only in the internal variant — it was
excluded at *extraction* time, not filtered after the fact, so an MCP
server pointed at `build/dita2graph-public` structurally cannot answer
questions about it (§6.1: "a topic that was never extracted cannot
leak").

## Part 2 — installing on your own, existing DITA project

Everything above ran through the Gradle harness for convenience. Here's
the same pipeline against a real project you already have, using
DITA-OT's plugin mechanism directly (§2, §15 Appendix A of
`docs/plugin-specification.md`).

### 2.1 Package and install the plugin

DITA-OT's local plugin install expects a `.zip`, not a bare directory
(confirmed directly — installing a directory fails with a
`java.util.zip` error, see `docs/dev/phase-0-findings.md` finding 7):

```bash
cd plugin/org.dita.dita2graph
zip -r /tmp/org.dita.dita2graph.zip . -x '.gitignore'
dita --install /tmp/org.dita.dita2graph.zip
```

Confirm it registered:

```bash
dita --propertyfile /dev/null --format dita2graph --help 2>&1 | head -20
```

You should see the `args.dita2graph.*` parameters listed (`depth`,
`mcp`, `emit-graph-json`, `store`, `include-drafts` — §2.3).

### 2.2 Run it against your root map

```bash
dita \
  --input /path/to/your/root.ditamap \
  --format dita2graph \
  --output /path/to/output \
  --args.dita2graph.mcp=true
```

Add `--filter your-audience.ditaval` if your project uses DITAVAL
profiling and you want a scoped bundle (public-facing docs only, one
product line only, etc. — same mechanism as Part 1.6).

Useful `--args.dita2graph.*` overrides for a large, real project:

- `depth=<N>` — cap how many levels of map containment become `contains`
  edges, instead of the `unlimited` default, if your map nests deeply
  and you only care about the top few levels.
- `include-drafts=true` — include `status="draft"` topics (excluded by
  default).
- `mcp=true` — write `mcp/mcp-server.toml` so `dita2graph-mcp --config`
  can find the bundle without you having to remember its path (§5.4)
  — do this if you're setting up MCP access, as below.

Two things worth knowing before you point this at a large project:

- **Every run is a full rebuild.** Incremental rebuild keyed on source
  file hashes is a real, designed feature (§3.3) but it's deferred to
  Phase 6+ (see `Roadmap.md`) — not implemented yet. Budget build time
  accordingly on a large corpus.
- **A broken `href` fails the build outright** (DITA-OT's own
  `[DOTX008E]`), which is a feature, not a bug (§9.3) — it means a graph
  never gets built from source with a genuinely dead cross-reference.
  An unresolved `keyref`, though, is only a warning
  (`[DOTJ047I]`/`DITA2GRAPH060W` for `<navref>`) and the build succeeds
  with that edge silently dropped — worth knowing if your project
  authors against a partial key space.

### 2.3 Validate the bundle

```bash
dita2graph-core validate --bundle /path/to/output/okf
```

### 2.4 Register the MCP server for this project

```bash
claude mcp add dita2graph -- dita2graph-mcp --config /path/to/output/mcp/mcp-server.toml
```

`--config` reads that file's `graph.okf` path and resolves the bundle
root from it, so you don't have to hand-type the bundle path again
every time it changes.

From here, Part 1.5's table of example questions applies the same way
— just substitute your own project's real topic titles and IDs for
`installing-product`/`configuration`.

## Troubleshooting

- **"Unsupported option" on an `--args.dita2graph.*` flag** — you're
  running against a DITA-OT that doesn't have the plugin installed, or
  installed an older copy of it; re-run the install step in 2.1.
- **`search_content` says no `rag/chunks.jsonl` found** — that content
  index is written by `dita2graph-core build` (or the transtype run
  itself), not by `validate` alone; re-run the extraction.
- **A question you expect to route through a tool instead reads a raw
  file** — check the server actually registered (`claude mcp list`)
  and that you're pointing it at the right bundle root, not a stale one
  from an earlier run.
- **`store` parameter doesn't seem to do anything** — correct for now:
  per `plugin.xml`, `sqlite`/`rocksdb` backends are planned (§7) but not
  implemented; `none` is the only value that does anything today, which
  is also effectively the default behavior.

## See also

- [`docs/plugin-specification.md`](plugin-specification.md) — full
  design spec and source of truth, especially §5.2 (tool argument
  shapes) and §15 (the condensed version of this quickstart).
- [`docs/architecture.md`](architecture.md) — component/class/activity/
  sequence diagrams.
- [`README.md`](../README.md#use-cases-with-ai-tools-claude-code) — the
  use-case sections this tutorial puts into practice.
