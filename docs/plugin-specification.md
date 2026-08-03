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

### 1.1 Target versions

DITA2Graph is specified against a fixed baseline so extraction behavior,
key resolution, and OKF output are reproducible across environments:

| Dependency | Version | Notes |
|---|---|---|
| DITA-OT | **4.4** | Latest stable line; matches the versions already validated by `dita-ot-gradle` (§7). |
| DITA standard | **1.3** | OASIS DITA 1.3 (topic types, key scopes, `conkeyref`, branch filtering). The extraction layer targets 1.3 markup; DITA 2.0-only constructs are out of scope for the MVP. |
| OKF standard | **v0.2** | [OKF v0.2 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md) — see §4.1 for how DITA2Graph maps onto it. |
| MCP | 2024-11-05 protocol revision | JSON-RPC 2.0 over stdio (HTTP transport planned, §10). |

Pinning DITA-OT 4.4 and DITA 1.3 means the normalized model in §3.2 can
assume 1.3 semantics for `conkeyref`, key scopes, and branch filtering
(`ditaval` + `<data>`-based conditions) without version-sniffing the source
map.

### 1.2 High-level architecture

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
- Write outputs (`okf/` bundle, `graph.json`, MCP server bundle) to the
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
 ├── okf/                   # OKF v0.2 knowledge bundle (markdown + YAML frontmatter, §4)
 │   ├── okf.toml             # Bundle config: okf_version, generator identity
 │   ├── index.md              # Progressive-disclosure directory listing
 │   ├── log.md                  # Chronological history of graph regenerations
 │   ├── maps/
 │   │   └── user-guide.md         # One concept per ditamap
 │   └── topics/
 │       ├── installing-product.md # One concept per DITA topic
 │       └── configuration.md
 ├── graph.json              # Flattened nodes+edges view of the bundle, for tooling/debug
 ├── graph.db                  # SQLite/RocksDB index built from okf/ (fast MCP queries)
 └── mcp/
     ├── mcp-server.toml    # MCP server configuration bound to graph.db + okf/
     └── manifest.json      # Declared resources & tools (see section 5)
```

The **bundle** (`okf/`) is the portable, human-readable, git-diffable
artifact — plain markdown, per the OKF v0.2 spec. `graph.db` is a derived,
disposable index the MCP server queries for speed; it can always be
rebuilt from `okf/` alone, the same relationship `okf-rs` itself uses
between its bundle and its `okf-search`/`okf-graph` indices.

---

## 3. DITA2Graph Core Engine

The DITA-OT plugin is intentionally a thin adapter. The heavy lifting
(graph construction, relation inference, OKF serialization, storage) lives
in a separate, language-agnostic **core engine** written in Rust, so it can
be reused outside of DITA-OT (e.g. as a standalone CLI or embedded in the
MCP server) and benefit from Rust's performance and memory-safety for
processing large documentation sets.

Rather than building an OKF writer and MCP server from scratch, the core
engine is designed to sit on top of the crates already published in
[`jyjeanne/okf-rs`](https://github.com/jyjeanne/okf-rs), which is itself
an OKF v0.2-conformant Rust toolchain and — notably — already ships an
**`okf-dita` crate** ("DITA XML converter: export an OKF bundle to DITA
topics, and import an existing DITA corpus as Document concepts"). That
crate is the natural starting point for the DITA→OKF direction of
DITA2Graph, rather than a parallel reimplementation:

| `okf-rs` crate | Reused for |
|---|---|
| `okf-dita` | DITA XML ↔ OKF concept conversion (topics → concepts, maps → index) |
| `okf-core` | Bundle config (`okf.toml`), path/type conventions |
| `okf-parser` | Reading frontmatter + body back out of a bundle |
| `okf-generator` | Writing conformant markdown + YAML frontmatter concepts |
| `okf-graph` | Call/reference-graph construction and traversal (`graph_*` queries) |
| `okf-validator` | Schema and link-integrity validation of the emitted bundle |
| `okf-search` | Free-text and ranked (BM25) search over the bundle |
| `okf-mcp` | JSON-RPC-over-stdio MCP server exposing the above as tools (§5.5) |

DITA2Graph's own code is then mostly the DITA-specific extraction (§3.2)
and the DITA relation taxonomy (§4.2) layered on top — relation inference,
`conref`/`conkeyref` dedup, and audience/product/key mapping onto OKF
frontmatter (§4.1) — plus the DITA-specific MCP tools in §5.2.

### 3.1 Architecture

```
DITA-OT (Java)
       |
       | normalized DITA model (JSON over stdin/IPC)
       v
dita2graph-core (Rust)
       |
       | DITA relation inference + conref/key dedup      (dita2graph-specific)
       v
okf-dita / okf-generator (from okf-rs)                    (reused)
       |
       | writes conformant OKF v0.2 bundle
       v
okf/ bundle (markdown + YAML frontmatter)
       |
       | okf-parser + okf-graph + okf-search index
       v
Storage (SQLite / RocksDB) + graph.json + okf-mcp server
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
- **OKF serialization**: emit one conformant OKF v0.2 concept document
  (markdown + YAML frontmatter) per topic/map into the `okf/` bundle,
  via `okf-generator` (see section 4).
- **Storage**: persist the derived query index to SQLite (default,
  zero-ops, good for most doc sets) or RocksDB (for very large graphs /
  high write throughput). The bundle itself needs no database — it's
  markdown on disk.

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

### 4.1 What OKF v0.2 actually is

Per the [OKF v0.2 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md),
OKF is **not** a JSON graph format — it is deliberately minimal: a
**bundle** is a directory tree of UTF-8 markdown files (**concepts**),
each with a YAML frontmatter block followed by a markdown body. There is
no schema registry and no required runtime: "if you can `cat` a file, you
can read OKF." DITA2Graph targets this directly rather than inventing its
own envelope.

Only one frontmatter key is required — `type` — everything else
(`title`, `description`, `resource`, `tags`, provenance, trust, lifecycle)
is optional and consumers must tolerate unknown/missing keys gracefully
(spec §11, "graceful degradation"). DITA2Graph's mapping from DITA onto
OKF frontmatter:

| DITA source | OKF frontmatter field |
|---|---|
| Topic type (`concept`/`task`/`reference`/`glossentry`) | `type` (e.g. `Concept`, `Task`, `Reference`, `Glossary Entry`) |
| `<title>` | `title` |
| `<shortdesc>`/`<abstract>` | `description` |
| Source `.dita` file path | `resource` |
| `audience`, `platform`, `product`, `otherprops` | `tags` |
| `conref`/`conkeyref` origin, resolved `keyref` | `sources` (provenance, §5.1 of the spec) |
| `dita2graph-core` version + generation timestamp | `generated: { by, at }` |
| Relation taxonomy edges (§4.2) not covered by a plain link | `relations` — a DITA2Graph **extension** field (spec explicitly allows producer-defined keys) |

Structural relationships that OKF *does* define natively — a standard
markdown link from one concept to another — are used wherever the DITA
relation is a simple reference (`references`, `related-to`,
`generated-from`). The typed, directional relations DITA is stricter about
(`requires`, `applies-to`, `contains`) are additionally captured in the
`relations` frontmatter extension so a consumer doesn't have to
regex-parse the body to recover them.

### 4.2 Conceptual model

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

### 4.3 Relationship taxonomy

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

### 4.4 OKF concept examples

`okf/topics/installing-product.md`, generated from the task topic in
§3.2:

````markdown
---
type: Task
title: Installing Product
description: Steps to install the product in a production environment.
resource: topics/installing-product.dita
tags: [admin, enterprise, install-task]
sources:
  - id: config-concept
    resource: okf/topics/configuration.md
    title: Configuration Overview
generated:
  by: dita2graph-core/0.1.0
  at: 2026-08-03T00:00:00Z
relations:
  requires: [configuration]
  contains: [installing-product-prereqs]
---

# Summary

Steps to install the product in a production environment.

# Requires

- [Configuration Overview](../topics/configuration.md)

# Contains

- [Installing Product: Prerequisites](installing-product-prereqs.md)
````

`okf/topics/configuration.md`, a plain concept topic with no extension
fields — still fully conformant per §11 of the spec, since `type` is the
only required key:

````markdown
---
type: Concept
title: Configuration Overview
resource: topics/configuration.dita
---

Configuration overview content goes here.
````

`okf/maps/user-guide.md` — the ditamap itself becomes a concept whose
`contains` relations mirror the map's `topicref` hierarchy, giving an
agent a single entry point to traverse the whole bundle:

````markdown
---
type: DITA Map
title: User Guide
resource: user-guide.ditamap
generated:
  by: dita2graph-core/0.1.0
  at: 2026-08-03T00:00:00Z
relations:
  contains: [installing-product, configuration]
---

# Topics

- [Installing Product](../topics/installing-product.md)
- [Configuration Overview](../topics/configuration.md)
````

`graph.json` (derived, not authoritative) still gives tooling a flattened
nodes/edges view for debugging without walking the bundle:

```json
{
  "nodes": [
    { "id": "installing-product", "type": "Task" },
    { "id": "configuration", "type": "Concept" }
  ],
  "edges": [
    { "from": "installing-product", "to": "configuration", "relation": "requires" }
  ]
}
```

### 4.5 What is preserved that a text/RAG pipeline loses

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
okf = "output/okf"

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

### 5.5 Reference implementation pattern (adapted from `jyjeanne/okf-rs`)

Rather than designing the MCP transport from scratch, `dita2graph-mcp`
follows the same minimal pattern already implemented and tested in the
`okf-mcp` crate of [`jyjeanne/okf-rs`](https://github.com/jyjeanne/okf-rs/tree/main/crates/okf-mcp):
JSON-RPC 2.0 over stdio, one message per line, with `stdout` reserved
exclusively for protocol messages and all diagnostics on `stderr`.

`crates/okf-mcp/src/main.rs` (excerpt, `okf-rs`, MIT/Apache-2.0):

```rust
//! Minimal Model Context Protocol (MCP) server exposing okf-rs bundle
//! queries (search, call graph, API surface) to AI agents.
//!
//! Speaks JSON-RPC 2.0 over stdio, one message per line (MCP's stdio
//! transport): requests are read from stdin, responses written to stdout.
//! Notifications (messages with no `id`) never get a response, even on
//! error, per the JSON-RPC spec. All non-protocol output (parse errors,
//! diagnostics) goes to stderr — stdout is reserved for protocol messages
//! only, since a stray `println!` would corrupt the stream for whatever
//! is reading it.

mod tools;

use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() -> Result<()> {
    let project_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let bundle = okf_core::config::resolve_bundle(&project_root, None);

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(line)?;
        if let Some(response) = handle_message(&request, &bundle) {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Dispatches one JSON-RPC message; returns `None` for notifications
/// (messages with no `id`), which never get a response.
fn handle_message(request: &Value, bundle: &std::path::Path) -> Option<Value> {
    let method = request.get("method")?.as_str()?;
    let id = request.get("id").cloned();

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "dita2graph-mcp", "version": env!("CARGO_PKG_VERSION") },
        })),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match tools::call(name, &arguments, bundle) {
                Ok(text) => Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": false })),
                Err(e) => Ok(json!({ "content": [{ "type": "text", "text": e.to_string() }], "isError": true })),
            }
        }
        "notifications/initialized" | "notifications/cancelled" => return None,
        other => Err(format!("method not found: {other}")),
    };

    let id = id?;
    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(message) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } }),
    })
}
```

`crates/okf-mcp/src/tools.rs` (excerpt) shows the shape every tool
declaration follows — a `tools/list` entry with a JSON-Schema
`inputSchema`, dispatched by name in `tools::call`:

```rust
/// The MCP `tools/list` result: name, human-readable description, and a
/// JSON Schema for the arguments `tools/call` expects.
pub fn list() -> Vec<Value> {
    vec![
        json!({
            "name": "search",
            "description": "Free-text search over the knowledge bundle by symbol, package, module, type, or tag.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Search text" } },
                "required": ["query"],
            },
        }),
        json!({
            "name": "graph_callers",
            "description": "List concepts that directly reference the given concept id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Concept id (find it with the search tool)" } },
                "required": ["id"],
            },
        }),
        // dita2graph-mcp would declare search_topics, find_related_topics,
        // explain_task, trace_dependencies, generate_summary here, in the
        // same shape (§5.2), dispatched to dita2graph-core instead of
        // okf-rs's generic okf-query layer.
    ]
}
```

`dita2graph-mcp` reuses this file-for-file: swap `okf-query`'s generic
graph/search calls for `dita2graph-core`'s DITA-relation-aware
equivalents, add the DITA-specific tool names from §5.2, and the
transport/dispatch code is unchanged. Each tool call re-reads the bundle
fresh (as `okf-mcp` does), so a running server always reflects the latest
`dita2graph` regeneration without needing a restart.

---

## 6. Recommended implementation stack

| Component | Technology |
|---|---|
| DITA processing | DITA-OT 4.4 |
| DITA standard | DITA 1.3 |
| Plugin (DITA-OT integration) | Java / Ant / Gradle |
| Build automation | Gradle (via `dita-ot-gradle`, see section 7) |
| Graph engine | Rust, built on `okf-rs` crates (`okf-dita`, `okf-core`, `okf-generator`, `okf-graph`, `okf-validator`, `okf-search`) |
| Knowledge format | OKF v0.2 |
| Agent interface | MCP (JSON-RPC 2.0, protocol rev. 2024-11-05), pattern from `okf-mcp` |
| Storage | SQLite / RocksDB (derived index only — the bundle itself is markdown) |
| CLI | Rust (Clap) |
| Serialization | Markdown + YAML frontmatter (bundle) / JSON (derived `graph.json`) |
| MCP transport | stdio (local) / HTTP (remote, planned) |

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

### 8.1 Comparison with standard RAG

| Dimension | Standard RAG (text → chunks → embeddings) | DITA2Graph (DITA → OKF → MCP) |
|---|---|---|
| Unit of retrieval | Arbitrary N-token text chunk, often splitting mid-thought | A whole DITA concept (topic, one coherent unit the author already scoped) |
| Relationships | Inferred at query time from embedding similarity | Explicit, author-declared (`reltable`, `keyref`, `xref`) plus a typed taxonomy (§4.3) |
| Reuse (conref) | Duplicated across every chunk that includes it, inflating the index and risking drift between copies | Single canonical concept, referenced by `sources`/relations — one source of truth |
| Applicability (audience/product/version) | Usually lost, or hacked in as metadata filters bolted onto the vector store | First-class (`tags`, DITAVAL-driven), enforced at extraction time |
| Answer for "what does X require?" | Best-effort: retrieve top-k similar chunks, hope one mentions a dependency | Deterministic: one graph traversal (`requires` edge) |
| Freshness / staleness detection | Re-embed everything, or nothing | Incremental rebuild keyed by source file hash (§3.3); `generated.at` frontmatter makes staleness inspectable |
| Failure mode on bad input | Silent — a broken doc still embeds and gets retrieved | Loud — `okf-validator` and DITA-OT's own key/conref resolution reject broken source before a graph is built (§8.3) |
| Output artifact | Vectors in a proprietary store; not human-readable | Plain markdown bundle; human-readable, diffable, greppable without tooling |

### 8.2 Reduced token consumption

A RAG pipeline typically has to over-retrieve to compensate for imprecise
similarity search — pulling in several chunks per query and letting the
model sort out which parts are relevant, which means every question costs
several thousand tokens of context just to answer something like "what
does this task require?" `okf-rs`'s own MCP tools demonstrate the
alternative: a graph query like `graph_callers`/`find_related_topics`
returns *only* the answer — on the order of tens of tokens, not the
thousands a full-file or multi-chunk read would cost — because the graph
edge already encodes the answer; the agent doesn't have to re-derive it
from prose. The same holds for `search_topics`/`trace_dependencies` in
DITA2Graph's tool set (§5.2): a typed lookup replaces a broad semantic
search plus several speculative chunk reads.

### 8.3 Validation

Because the graph is built from DITA-OT's own preprocessing pipeline
rather than scraped from rendered output, DITA2Graph gets validation for
free at multiple layers before an agent ever sees the data:

- **DITA-OT key/conref resolution** fails the build on unresolved
  `keyref`/`conkeyref` — a broken reference can't silently make it into
  the graph the way a broken link can silently make it into a RAG index.
- **`dita-ot-gradle`'s `DitaOtValidateTask`/`DitaLinkCheckTask`** (§7)
  catch XML validity, reference integrity, and dead links *before*
  `dita2graph` runs, gating graph generation on green validation.
- **`okf-validator`** checks the emitted bundle for OKF v0.2 schema
  conformance and link integrity (every `relations`/markdown-link target
  actually resolves to a concept in the bundle) — the graph can't claim a
  relationship to a concept that doesn't exist.
- **Typed edges are structurally checked**, not inferred: a `requires`
  edge exists only if the source DITA actually declared that
  relationship, so there's no equivalent of an embedding model
  hallucinating a similarity that isn't really there.

### 8.4 Native AI-tool interaction

MCP gives the agent a typed contract instead of a single fuzzy
`search(query: string)` endpoint: resources are addressable by stable
URI (`dita://topic/{id}`), and each tool declares its own JSON-Schema
input (§5.2, §5.5) — so a client like Claude Code knows in advance what
arguments `trace_dependencies` needs and what shape its result takes,
instead of prompting the model to free-form a search string and parse
unstructured prose back out. This also composes: an agent can chain
`search_topics` → `find_related_topics` → `explain_task` in a single
turn, each call narrowing on IDs the previous call returned, which a flat
vector index has no equivalent for since it has no notion of an "ID"
to traverse from.

---

## 9. MVP scope

For a first minimum viable product, pinned to DITA-OT 4.4 / DITA 1.3 /
OKF v0.2:

1. DITA-OT plugin (`org.dita.dita2graph`) registering a new transtype and
   extracting the normalized DITA model described in section 3.2.
2. Extraction of DITA map/topic relationships (`contains`, `references`,
   `requires`, `generated-from`) into the OKF model, reusing/adapting
   `okf-dita` and `okf-generator` from `jyjeanne/okf-rs` (section 3) for
   the actual bundle-writing rather than hand-rolling an OKF writer.
3. Generation of a conformant `okf/` bundle (markdown + YAML frontmatter,
   section 4) plus the derived `graph.json`.
4. A Rust MCP server exposing the OKF graph via the resources/tools in
   section 5, following the `okf-mcp` JSON-RPC-over-stdio pattern
   (section 5.5).
5. A `build.gradle` example (section 7) wiring `dita-ot-gradle` to
   download DITA-OT 4.4, install the plugin, validate content
   (`okf-validator` + `DitaOtValidateTask`/`DitaLinkCheckTask`), and run
   the `dita2graph` transtype as part of a normal CI build.

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
