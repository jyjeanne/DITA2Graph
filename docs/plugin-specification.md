# DITA2Graph Plugin Specification

## 1. Overview

**DITA2Graph** is a DITA-OT plugin that converts DITA content into a semantic
knowledge graph using the **Open Knowledge Format (OKF)** as its
representation model, and exposes that graph to AI agents through the
**Model Context Protocol (MCP)**.

The core idea: DITA-OT already understands the *meaning* of technical
documentation — topic types, maps, key definitions, conref reuse, audience
and product filtering, cross-references. Most AI/RAG pipelines throw all of
that away and reduce documentation to flat text chunks:

```
PDF/HTML → Text → Chunks → Embeddings → RAG
```

DITA2Graph instead preserves DITA's structural semantics all the way through
to the AI layer:

```
DITA → Semantic Graph (OKF) → MCP → AI Agent
```

### 1.1 High-level architecture

```
DITA Repository
       |
       v
+----------------+
|    DITA-OT     |
|   Pipeline     |
+----------------+
       |
       | plugin extension
       v
+----------------+
|   DITA2Graph   |
|  DITA Extractor|
+----------------+
       |
       v
+----------------+
|  OKF Knowledge |
|     Graph      |
+----------------+
       |
+------+------+
|             |
v             v
OKF Files/API   MCP Server
                  |
                  v
        Claude Code / AI Agents
```

The system is composed of three layers:

1. **DITA-OT Plugin Layer** — hooks into the DITA-OT build pipeline and
   extracts a normalized DITA model.
2. **DITA2Graph Core Engine** — transforms the normalized model into an OKF
   knowledge graph.
3. **MCP Server Layer** — exposes the OKF graph as resources and tools that
   AI agents can query.

---

## 2. DITA-OT Plugin Layer

The plugin behaves like a standard DITA-OT extension, installed via the
DITA-OT plugin mechanism (`dita --install`) and registered as a new
transformation type.

### 2.1 Plugin layout

```
org.dita.dita2graph/
│
├── plugin.xml
├── build.xml
├── integrator.xml
├── config/
│   └── dita2graph.xml
│
├── lib/
│   └── dita2graph-core.jar
│
└── bin/
    └── dita2graph
```

- **plugin.xml** — declares the `dita2graph` transtype, extension points,
  and Ant targets contributed to the DITA-OT pipeline.
- **build.xml** — Ant build wiring the plugin's preprocessing and
  post-processing steps into the DITA-OT `preprocess`/`compile` phases.
- **integrator.xml** — registers the plugin's parameters (feature flags,
  output paths, OKF options) so they are recognized by `dita` CLI/Gradle
  invocations.
- **config/dita2graph.xml** — default configuration (graph depth, relation
  types to extract, MCP server settings).
- **lib/dita2graph-core.jar** — thin Java bridge that shells out to (or
  JNI/FFI-binds) the Rust core engine described in section 3.
- **bin/dita2graph** — standalone CLI entry point for running extraction
  outside of a full DITA-OT publish (e.g. in CI, or for incremental graph
  updates).

### 2.2 Responsibilities

- Hook into the DITA-OT preprocessing pipeline (after key-space resolution,
  before final output rendering) via a custom transtype.
- Receive the resolved DITA map, including:
  - resolved keys (`keyref`, `keydef`) and key scopes,
  - resolved `conref`/`conkeyref` content,
  - resolved `topicref` hierarchy, including `chunk`, `href`, `format`,
  - filtering/flagging (`audience`, `platform`, `product`, `otherprops`)
    via DITAVAL,
  - metadata (`prolog`, `metadata`, `resourceid`).
- Normalize this into a DITA semantic model (section 2.3) independent of
  DITA-OT's internal XML representation.
- Invoke the DITA2Graph core engine to build the OKF graph.
- Write outputs (`knowledge.okf`, `graph.json`, MCP server bundle) to the
  DITA-OT output directory.

### 2.3 Invocation

```bash
dita \
  --input user-guide.ditamap \
  --format dita2graph \
  --filter audience-admin.ditaval \
  --args.dita2graph.depth=3 \
  --args.dita2graph.mcp=true
```

Key parameters:

| Parameter | Default | Description |
|---|---|---|
| `args.dita2graph.depth` | `unlimited` | Max relationship traversal depth captured in the graph |
| `args.dita2graph.mcp` | `false` | Whether to also emit an MCP server bundle |
| `args.dita2graph.format` | `json` | OKF serialization: `json` \| `yaml` |
| `args.dita2graph.store` | `sqlite` | Backing store for the generated graph: `sqlite` \| `rocksdb` \| `none` |
| `args.dita2graph.include-drafts` | `false` | Include topics flagged `status="draft"` |

### 2.4 Output

```
output/
 ├── knowledge.okf        # OKF knowledge graph (JSON-LD-style envelope)
 ├── graph.json            # Flattened graph (nodes + edges) for tooling/debug
 ├── graph.db               # SQLite/RocksDB store (if args.dita2graph.store != none)
 └── mcp/
     ├── mcp-server.toml    # MCP server configuration bound to graph.db
     └── manifest.json      # Declared resources & tools (see section 4)
```

---

## 3. DITA2Graph Core Engine

The DITA-OT plugin is intentionally a thin adapter. The heavy lifting
(graph construction, relation inference, OKF serialization, storage) lives
in a separate, language-agnostic **core engine** written in Rust, so it can
be reused outside of DITA-OT (e.g. as a standalone CLI or embedded in the
MCP server) and benefit from Rust's performance and memory-safety for
processing large documentation sets.

### 3.1 Architecture

```
DITA-OT (Java)
       |
       | normalized DITA model (JSON over stdin/IPC)
       v
dita2graph-core (Rust)
       |
       | graph construction + relation inference
       v
OKF Generator
       |
       v
Storage (SQLite / RocksDB) + graph.json + knowledge.okf
```

The Java plugin and Rust engine communicate over a small JSON-over-stdio (or
Unix domain socket, for long-running incremental mode) protocol, so the two
layers can evolve independently and the core engine can be tested and
versioned on its own.

### 3.2 Normalized DITA model (input to the core engine)

```json
{
  "type": "topic",
  "id": "installing-product",
  "topicType": "task",
  "title": "Installing Product",
  "shortdesc": "Steps to install the product in a production environment.",
  "audience": ["admin"],
  "product": ["enterprise"],
  "keys": ["install-task"],
  "sourceFile": "topics/installing-product.dita",
  "links": [
    {
      "relation": "requires",
      "target": "configuration"
    },
    {
      "relation": "contains",
      "target": "installing-product-prereqs"
    }
  ]
}
```

### 3.3 Core engine responsibilities

- **Graph construction**: build nodes for maps, topics, and topic-type
  subtypes (concept/task/reference/glossentry/…), and edges for every DITA
  relationship (section 3.4).
- **Relation inference**: derive implicit relationships DITA doesn't encode
  explicitly, e.g. "topics sharing a `product` value are `related-to`",
  or "a task's `<cmd>` referencing a `<uicontrol>` defined in a reference
  topic is `applies-to`".
- **Deduplication & reuse tracking**: because of `conref`/`conkeyref`, the
  same content fragment can appear in multiple topics — the engine tracks a
  single canonical node with multiple `contains` edges rather than
  duplicating content.
- **Incremental updates**: on re-run, diff against the existing
  `graph.db` and only recompute changed subgraphs (keyed by source file
  hash), so large doc sets don't require a full rebuild on every publish.
- **OKF serialization**: emit the graph as `knowledge.okf` (see section 4).
- **Storage**: persist to SQLite (default, zero-ops, good for most doc
  sets) or RocksDB (for very large graphs / high write throughput).

### 3.4 CLI (Rust, Clap-based)

The core engine also ships as a standalone binary, independent of DITA-OT,
useful for CI pipelines or ad-hoc graph inspection:

```bash
dita2graph-core build \
  --input output/normalized-model.json \
  --output output/ \
  --store sqlite \
  --format json

dita2graph-core query \
  --store output/graph.db \
  --topic installing-product \
  --relation requires
```

---

## 4. OKF Knowledge Graph Model

The OKF output preserves DITA concepts rather than flattening them into
prose, which is what gives downstream AI agents a semantic model instead of
opaque text chunks.

### 4.1 Conceptual model

```
Knowledge Base
|
+-- Product
|
+-- DITA Map
|
+-- Topics
|     |
|     +-- Concept
|     +-- Task
|     +-- Reference
|     +-- Glossary Entry
|
+-- Relationships
|
+-- Metadata
```

### 4.2 Relationship taxonomy

```
topic
 |
 +-- contains        (map/topic -> child topic, structural)
 |
 +-- references       (xref, related-links)
 |
 +-- related-to        (related-links role="related", inferred similarity)
 |
 +-- applies-to          (task/step -> UI element, config option, product)
 |
 +-- requires              (prerequisite relationship, e.g. task -> concept)
 |
 +-- generated-from          (topic -> conref/conkeyref source fragment)
```

### 4.3 OKF envelope example

```json
{
  "okf_version": "1.0",
  "source": {
    "type": "dita-map",
    "path": "user-guide.ditamap",
    "generatedAt": "2026-08-03T00:00:00Z",
    "generator": "dita2graph-core/0.1.0"
  },
  "nodes": [
    {
      "id": "installing-product",
      "type": "Task",
      "title": "Installing Product",
      "audience": ["admin"],
      "product": ["enterprise"],
      "sourceFile": "topics/installing-product.dita"
    },
    {
      "id": "configuration",
      "type": "Concept",
      "title": "Configuration Overview"
    }
  ],
  "edges": [
    {
      "from": "installing-product",
      "to": "configuration",
      "relation": "requires"
    }
  ],
  "metadata": {
    "product": "enterprise",
    "audience": ["admin", "user"],
    "keyDefinitions": {
      "install-task": "installing-product"
    }
  }
}
```

### 4.4 What is preserved that a text/RAG pipeline loses

- Topic types (concept/task/reference/glossentry) and their semantics.
- Reuse relationships via `conref`/`conkeyref` (a single source of truth,
  not N duplicated chunks).
- Key spaces and key definitions (`keyref` resolution), including
  key-scoped overrides.
- Product/platform/audience variants and DITAVAL-driven conditional text.
- Explicit relationship tables (`<reltable>`) rather than inferred
  similarity only.
- Applicability (which content applies to which product/version/audience).

---

## 5. MCP Server Layer

The MCP server exposes the OKF graph to AI agents as typed **resources**
and callable **tools**, so an agent queries the graph directly instead of
performing semantic search over raw text.

### 5.1 Resources

```
dita://topics
dita://topic/{id}
dita://product/{name}
dita://architecture
dita://map/{mapId}
dita://relation/{topicId}/{relationType}
```

### 5.2 Tools

```
search_topics(query, topicType?, audience?, product?)
find_related_topics(topicId, relation?)
explain_task(topicId)
trace_dependencies(topicId, depth?)
generate_summary(topicId | mapId)
list_key_definitions(scope?)
```

### 5.3 Example interaction

```
User:
"How do I configure authentication?"

Claude Code:
  calls dita2graph.search_topics(query="authentication configuration")

returns:
  Topic: Authentication Configuration (Concept)
  Requires:
    - Security Module (Concept)
    - User Database (Reference)
  Related tasks:
    - Configuring SSO (Task)
    - Rotating API Keys (Task)
```

### 5.4 Server configuration

The plugin emits `mcp/mcp-server.toml`, generated from `config/dita2graph.xml`:

```toml
[server]
name = "dita2graph"
transport = "stdio"   # or "http" for remote/multi-client deployments

[graph]
store = "output/graph.db"
okf = "output/knowledge.okf"

[resources]
enable = ["topics", "product", "architecture", "map", "relation"]

[tools]
enable = ["search_topics", "find_related_topics", "explain_task", "trace_dependencies", "generate_summary"]
```

Running it:

```bash
dita2graph-mcp serve --config output/mcp/mcp-server.toml
```

which registers as a normal MCP server for Claude Code / Claude Desktop /
any MCP-compatible client via stdio or HTTP transport.

---

## 6. Recommended implementation stack

| Component | Technology |
|---|---|
| DITA processing | DITA-OT |
| Plugin (DITA-OT integration) | Java / Ant / Gradle |
| Build automation | Gradle (via `dita-ot-gradle`, see section 7) |
| Graph engine | Rust |
| Knowledge format | OKF |
| Agent interface | MCP |
| Storage | SQLite / RocksDB |
| CLI | Rust (Clap) |
| Serialization | JSON / YAML |
| MCP transport | stdio (local) / HTTP (remote) |

---

## 7. Build integration with Gradle (`dita-ot-gradle`)

For teams that already build their DITA output with Gradle rather than
invoking `dita` directly, DITA2Graph is designed to slot into
[`jyjeanne/dita-ot-gradle`](https://github.com/jyjeanne/dita-ot-gradle) — a
Gradle plugin (`io.github.jyjeanne.dita-ot-gradle`) that manages DITA-OT
itself: downloading it, installing plugins into it (including
`org.dita.dita2graph`), validating content, and running transformations,
all as ordinary Gradle tasks with up-to-date checking and configuration
cache support.

### 7.1 Why use it

- **No manual DITA-OT install.** `DitaOtDownloadTask` fetches and verifies a
  pinned DITA-OT release, so CI machines and developer laptops build against
  an identical toolkit version.
- **Plugin installation as a build step.** `DitaOtInstallPluginTask` can
  install `org.dita.dita2graph` from a local path, URL, or registry entry
  as part of `./gradlew build`, so the graph-generation plugin doesn't need
  to be pre-installed on every machine.
- **Validation before graph generation.** `DitaOtValidateTask` and
  `DitaLinkCheckTask` catch broken conrefs, key references, and dead links
  *before* DITA2Graph runs, so the knowledge graph is never built from
  broken source content.
- **Incremental builds.** Gradle's configuration cache (77% faster
  up-to-date builds per the plugin's own benchmarks) means re-running the
  graph extraction on an unchanged doc set is close to free.
- **Multiple output targets in one build.** The same `DitaOtTask` type used
  for `html5`/`pdf` output can drive the `dita2graph` transtype alongside
  normal publishing outputs.

### 7.2 Example `build.gradle`

```groovy
plugins {
    id 'io.github.jyjeanne.dita-ot-gradle' version '2.8.6'
}

tasks.register('downloadDitaOt', com.github.jyjeanne.DitaOtDownloadTask) {
    version = '4.2.3'
}

tasks.register('installDita2Graph', com.github.jyjeanne.DitaOtInstallPluginTask) {
    dependsOn downloadDitaOt
    ditaOtDir = layout.buildDirectory.dir('dita-ot/dita-ot-4.2.3')
    plugins = ['org.dita.dita2graph']   // local path, URL, or registry id
    force = false
}

tasks.register('validateDocs', com.github.jyjeanne.DitaOtValidateTask) {
    dependsOn installDita2Graph
    ditaOt layout.buildDirectory.dir('dita-ot/dita-ot-4.2.3')
    input 'docs/user-guide.ditamap'
}

tasks.register('checkLinks', com.github.jyjeanne.DitaLinkCheckTask) {
    dependsOn validateDocs
    ditaOt layout.buildDirectory.dir('dita-ot/dita-ot-4.2.3')
    input 'docs/user-guide.ditamap'
}

tasks.register('buildKnowledgeGraph', com.github.jyjeanne.DitaOtTask) {
    dependsOn checkLinks
    ditaOt layout.buildDirectory.dir('dita-ot/dita-ot-4.2.3')
    input 'docs/user-guide.ditamap'
    output 'build/dita2graph'
    transtype 'dita2graph'

    properties {
        'args.dita2graph.mcp' to 'true'
        'args.dita2graph.store' to 'sqlite'
        'args.dita2graph.depth' to '3'
    }

    progressStyle 'DETAILED'
}

// Normal HTML5 publishing can run alongside graph generation
tasks.register('publishHtml', com.github.jyjeanne.DitaOtTask) {
    dependsOn checkLinks
    ditaOt layout.buildDirectory.dir('dita-ot/dita-ot-4.2.3')
    input 'docs/user-guide.ditamap'
    output 'build/docs/html'
    transtype 'html5'
}

tasks.register('publishAll') {
    dependsOn buildKnowledgeGraph, publishHtml
}
```

Kotlin DSL equivalent for the graph-generation task:

```kotlin
tasks.register<com.github.jyjeanne.DitaOtTask>("buildKnowledgeGraph") {
    dependsOn("checkLinks")
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-4.2.3"))
    input("docs/user-guide.ditamap")
    output("build/dita2graph")
    transtype("dita2graph")

    properties {
        "args.dita2graph.mcp" to "true"
        "args.dita2graph.store" to "sqlite"
    }
}
```

Run it:

```bash
./gradlew buildKnowledgeGraph
# or, to validate, publish HTML, and build the graph together
./gradlew publishAll
```

### 7.3 CI recommendation

```properties
# gradle.properties
org.gradle.configuration-cache=true
org.gradle.parallel=true
```

Use `progressStyle = 'QUIET'` on `buildKnowledgeGraph` in CI so logs stay
readable, and gate `buildKnowledgeGraph` behind `validateDocs` and
`checkLinks` succeeding, so a broken source map never produces a stale or
partially-built graph that an AI agent could query with confidence it
doesn't deserve.

---

## 8. Why this architecture is interesting

The important innovation is that **DITA-OT already knows the meaning of
technical documentation**. Most AI systems reduce documentation to:

```
PDF/HTML → Text → Chunks → Embeddings → RAG
```

DITA2Graph instead does:

```
DITA → Semantic Graph → OKF → MCP → AI Agent
```

which preserves:

- topic types,
- reuse (conref/conkeyref),
- keys and key scopes,
- product/platform variants,
- audience and applicability filtering,
- explicit relationship tables.

This makes DITA2Graph closer to an **AI-native documentation compiler**
than a search index: an agent asking "how do I configure authentication?"
gets a typed graph traversal (task → requires → concept/reference) instead
of a best-effort nearest-neighbor match over disconnected text chunks.

---

## 9. MVP scope

For a first minimum viable product:

1. DITA-OT plugin (`org.dita.dita2graph`) registering a new transtype and
   extracting the normalized DITA model described in section 3.2.
2. Extraction of DITA map/topic relationships (`contains`, `references`,
   `requires`, `generated-from`) into the OKF model.
3. Generation of `knowledge.okf` (JSON/YAML) and `graph.json`.
4. A Rust MCP server exposing the OKF graph via the resources/tools in
   section 5.
5. A `build.gradle` example (section 7) wiring `dita-ot-gradle` to
   download DITA-OT, install the plugin, validate content, and run the
   `dita2graph` transtype as part of a normal CI build.

That scope is already a self-contained, demonstrable, open-source project:
DITA in, queryable knowledge graph and live MCP server out.

## 10. Future work

- Relation inference beyond explicit DITA markup (e.g. embedding-based
  `related-to` suggestions layered on top of the structural graph, clearly
  distinguished from author-declared relationships).
- Multi-map / multi-product graph federation (merging graphs from several
  `ditamap`s into one queryable knowledge base, e.g. per-product graphs
  joined into an org-wide graph).
- Graph versioning/diffing across doc releases, so an agent can answer
  "what changed about authentication between v3 and v4?".
- HTTP transport for the MCP server to support shared/remote deployments,
  with per-audience access control mirroring DITA's `audience` filtering.
- A DITA-OT PDF/HTML5 plugin variant that annotates rendered output with
  links back into the graph (e.g. "view related topics" panels driven by
  `dita2graph.find_related_topics`).
