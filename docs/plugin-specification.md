# DITA2Graph Plugin Specification

## Table of contents

1. [Overview](#1-overview)
2. [DITA-OT Plugin Layer](#2-dita-ot-plugin-layer)
3. [DITA2Graph Core Engine](#3-dita2graph-core-engine)
4. [OKF Knowledge Graph Model](#4-okf-knowledge-graph-model)
5. [MCP Server Layer](#5-mcp-server-layer)
6. [Security and Access Control](#6-security-and-access-control)
7. [Recommended Implementation Stack](#7-recommended-implementation-stack)
8. [Build Integration with Gradle](#8-build-integration-with-gradle-dita-ot-gradle)
9. [Why This Architecture Is Interesting](#9-why-this-architecture-is-interesting)
10. [Testing Strategy](#10-testing-strategy)
11. [MVP Scope](#11-mvp-scope)
12. [Development Phases](#12-development-phases)
13. [Future Work](#13-future-work)
14. [Licensing](#14-licensing)
15. [Appendix A: Quickstart](#15-appendix-a-quickstart)

---

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

### 1.1 Target versions and compatibility

DITA2Graph is specified against a fixed baseline so extraction behavior,
key resolution, and OKF output are reproducible across environments:

| Dependency | Version | Notes |
|---|---|---|
| DITA-OT | **4.4** | Latest stable line; matches the versions already validated by `dita-ot-gradle` (§8). |
| DITA standard | **1.3** | OASIS DITA 1.3 (topic types, key scopes, `conkeyref`, branch filtering). The extraction layer targets 1.3 markup; DITA 2.0-only constructs are out of scope for the MVP. |
| OKF standard | **v0.2** | [OKF v0.2 specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md) — see §4.1 for how DITA2Graph maps onto it. |
| MCP | 2024-11-05 protocol revision | JSON-RPC 2.0 over stdio (HTTP transport planned, §6.3/§13). |
| Gradle | **9.0 minimum** (currently building against 9.6.x) | Required, not just recommended — `dita-ot-gradle` itself only reaches "full support" at 9.0+ (§8); older Gradle isn't a supported target. |
| Java (toolkit + DITA2Graph's own Java code) | **25 (latest LTS)** | DITA-OT 4.4 itself only requires Java 17+; 25 is a floor DITA2Graph sets for its own code (`lib/dita2graph-core.jar`, once written) and for the Gradle build's toolchain, pinned in the repo-root `.java-version`. |
| Rust (`dita2graph-core`, `dita2graph-mcp`) | **Latest stable** (currently 1.97.1) | Pinned exactly in the repo-root `rust-toolchain.toml`; edition 2024, `rust-version = "1.97"` in `Cargo.toml`. Rust has no LTS concept — "latest stable" means re-pinning periodically (§6.5's pinning discipline), not a one-time floor. |

Pinning DITA-OT 4.4 and DITA 1.3 means the normalized model in §3.2 can
assume 1.3 semantics for `conkeyref`, key scopes, and branch filtering
(`ditaval` + `<data>`-based conditions) without version-sniffing the source
map. The Gradle/Java/Rust rows are toolchain floors, not content-processing
assumptions like the rows above them — they get bumped independently
(e.g. re-pinning Rust every so often) without touching DITA-OT/DITA/OKF/MCP
compatibility.

**Compatibility policy:** `org.dita.dita2graph`, `dita2graph-core`, and
`dita2graph-mcp` are versioned independently with semver, but released
together against a single tested DITA-OT/OKF pair per minor release (e.g.
`dita2graph 0.x` ↔ DITA-OT 4.4 ↔ OKF v0.2). A DITA-OT major-version bump or
an OKF spec bump each get their own compatibility row in this table and
their own migration note before being adopted as the new baseline — the
plugin does not silently follow upstream latest.

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

### 1.3 Glossary

| Term | Meaning |
|---|---|
| DITA | Darwin Information Typing Architecture — the OASIS XML standard for modular, reusable technical documentation. |
| DITA-OT | DITA Open Toolkit — the reference publishing engine that processes DITA source into output formats. |
| Transtype | A DITA-OT output-format target selected via `-f`/`--format` (e.g. `html5`, `pdf`, `dita2graph`). |
| Topic | The smallest reusable DITA content unit; typed as `concept`, `task`, `reference`, `glossentry`, etc. |
| Ditamap | The XML manifest that assembles topics (via `topicref`) into a publication. |
| Key / `keyref` / `keydef` | DITA's indirection mechanism: a `keydef` binds a key to a resource within a key scope; `keyref` resolves against it. |
| `conref` / `conkeyref` | Content-reuse mechanisms that pull a fragment from one topic into another, by ID or by key. |
| DITAVAL | A filter file (`.ditaval`) that includes/excludes/flags content by profiling attributes (`audience`, `platform`, `product`, `otherprops`) during processing. |
| OKF | Open Knowledge Format — a minimal markdown-plus-YAML-frontmatter convention for portable, agent- and human-readable knowledge. |
| Bundle (OKF) | An OKF-conformant directory tree of concept documents; the unit of distribution. |
| Concept (OKF) | **Ambiguity warning:** in OKF, "concept" means *any* markdown document in a bundle, of any `type`. This is a different sense from the DITA `concept` topic type. DITA2Graph uses "OKF concept" or "concept document" when the OKF sense is meant, and "DITA `<concept>` topic" or `type: Concept` when the DITA topic type is meant. |
| Frontmatter | The YAML metadata block at the top of an OKF concept document. |
| MCP | Model Context Protocol — a JSON-RPC-based protocol for exposing typed resources and tools to AI agents. |

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
├── cfg/
│   ├── dita2graph.xml
│   └── messages.xml
│
├── java/                    # Gradle project that builds lib/dita2graph-core.jar
│   └── src/main/java/org/dita/dita2graph/tasks/...
│
├── lib/
│   └── dita2graph-core.jar   # built artifact, gitignored — see java/README.md
│
└── bin/
    └── dita2graph
```

- **plugin.xml** — declares the plugin id (`org.dita.dita2graph`), a
  top-level `<transtype name="dita2graph" desc="..."/>` element
  registering the transtype itself, and `<feature>` elements wiring
  `build.xml` into the Ant build (`ant.import`), the classpath
  (`dita.conductor.lib.import`), and the message catalog
  (`dita.xsl.messages`) — all four confirmed against DITA-OT 4.4's own
  bundled plugins and a live install/dispatch run (§12 Phase 0/1 status;
  `docs/dev/phase-0-findings.md` finding 5). There is **no**
  `dita.transtype`/`dita.conductor.transtype.check` extension point — an
  earlier draft of this spec invented both; neither exists in DITA-OT.
  The plugin does **not** ship a per-plugin `integrator.xml`: DITA-OT
  auto-generates one toolkit-wide `integrator.xml` by aggregating every
  installed plugin's `plugin.xml` during `dita --install`/`ant
  integrator` — an individual plugin only ever authors `plugin.xml` and
  (optionally) `build.xml`.
- **build.xml** — Ant build wiring the plugin's preprocessing and
  post-processing steps into the DITA-OT `preprocess`/`compile` phases,
  and invoking `lib/dita2graph-core.jar`. DITA-OT dispatches transtype
  `dita2graph` to an Ant target literally named `dita2dita2graph` (its
  own `dita2` + transtype-value convention, confirmed against
  `org.dita.html5`'s `html5` → `dita2html5`), depending on
  `build-init,preprocess2` — not just `preprocess` — since `build-init`
  is what defines `${dita.temp.dir}` and the Ant project references the
  map-first `preprocess2` pipeline needs (finding 5).
- **cfg/dita2graph.xml** — a documentation summary of the
  `args.dita2graph.*` defaults (graph depth, relation types, MCP server
  settings); DITA-OT does **not** read this file automatically (its
  `<dita2graph-config>` shape is a DITA2Graph convention, not a DITA-OT
  one) — the defaults that actually take effect are the plain Ant
  `<property>` declarations at the top of `build.xml`'s target, which
  this file's defaults must be kept in sync with by hand
  (`docs/dev/phase-0-findings.md` finding 6).
- **cfg/messages.xml** — the plugin's DITA-OT message catalog (§2.5):
  declares `DITA2GRAPHnnnX` message IDs with severity, so Java/Ant code
  raises consistent, documented diagnostics through DITA-OT's own logger
  instead of ad hoc `System.out` output.
- **java/** — a standalone Gradle project building `lib/dita2graph-core.jar`:
  `org.dita.dita2graph.tasks.ExtractTask` (the `dita2graph:extract` Ant
  task) and its helpers, which parse DITA-OT's resolved job data into
  the normalized model (§3.2) and shell out to the `dita2graph-core`
  Rust binary (§3.4). Covered by a real unit test against a fixture
  shaped like actual DITA-OT 4.4 output; see `java/README.md`.
- **lib/dita2graph-core.jar** — the thin Java bridge `java/` builds,
  shelling out to the Rust core engine described in section 3 (found via
  the `DITA2GRAPH_CORE_BIN` env var or `PATH` — not a repo-relative
  path, since an installed plugin zip has no `target/` directory of its
  own; bundling the platform-specific Rust binary for a real release
  remains Phase 4/5 work). Not committed — a gitignored build artifact,
  reproducible via `java/`.
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

**Filtering happens here, not later.** DITAVAL-driven exclusion must be
applied during this preprocessing step, before content ever reaches the
core engine — the plugin should never extract audience/product-restricted
content into the normalized model and rely on a downstream layer to hide
it. This is what lets a "public" and an "internal" OKF bundle be produced
from the same source by swapping the DITAVAL file, with no risk of
restricted content leaking into the public one (§6.1).

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
| `args.dita2graph.emit-graph-json` | `true` | Whether to also emit the derived `graph.json` flattened view alongside the OKF bundle (the bundle itself is always the markdown+YAML format defined by OKF v0.2 — this is not a format choice, see §4.1) |
| `args.dita2graph.store` | `sqlite` | Backing store for the generated query index: `sqlite` \| `rocksdb` \| `none` |
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

### 2.5 Error handling, logging, and exit codes

DITA2Graph follows DITA-OT's own diagnostic conventions rather than
inventing its own, so its messages show up consistently alongside other
plugin output in a normal `dita` build log:

- **Message catalog** (`cfg/messages.xml`): every diagnostic the plugin
  can raise gets a stable ID of the form `DITA2GRAPHnnnX`, where `X` is
  `F` (fatal), `E` (error), `W` (warning), or `I` (info) — the same
  scheme DITA-OT's own message catalog uses. Representative entries:

  | ID | Severity | Meaning |
  |---|---|---|
  | `DITA2GRAPH001E` | Error | Unresolved `keyref`/`conkeyref` encountered during extraction — build fails, since a graph built on an unresolved reference would be silently wrong. |
  | `DITA2GRAPH010W` | Warning | Ambiguous relation inference (e.g. two candidate `applies-to` targets) — the lower-confidence edge is dropped, not guessed. |
  | `DITA2GRAPH020I` | Info | Topic skipped because `status="draft"` and `args.dita2graph.include-drafts=false`. |
  | `DITA2GRAPH030E` | Error | Generated OKF concept failed `okf-validator` conformance — build fails before the bundle is considered complete (§6.4, §10). |
  | `DITA2GRAPH040W` | Warning | Topic has no resolvable `type` mapping (unknown/custom topic type) — emitted as a generic OKF concept per the spec's graceful-degradation rule (§4.1), but flagged so authors can review it. |
  | `DITA2GRAPH050E` | Error | A generated OKF concept matches a high-confidence secret pattern (AWS access key, PEM private key, GitHub/Slack token) — build fails, not a warning (§6.4). |

- **Exit codes**: `0` success; `1` validation failure (bad DITA input,
  failed `okf-validator` check — recoverable by fixing source content);
  `2` internal error (bug in the plugin/core engine itself). This mirrors
  the Ant/DITA-OT convention that any non-zero exit fails the enclosing
  build (and thus the Gradle task in §8).
- **Where diagnostics go**: messages raised inside the DITA-OT pipeline go
  through DITA-OT's own logger (so they appear in the standard build log
  and respect `-v`/log-level flags); `bin/dita2graph`/`dita2graph-core`
  running standalone (outside a `dita` invocation) log structured JSON to
  stderr instead, so CI can parse them without a DITA-OT log format
  dependency.

---

## 3. DITA2Graph Core Engine

The DITA-OT plugin is intentionally a thin adapter. The heavy lifting
(graph construction, relation inference, OKF serialization, storage) lives
in a separate, language-agnostic **core engine** written in Rust, so it can
be reused outside of DITA-OT (e.g. as a standalone CLI or embedded in the
MCP server) and benefit from Rust's performance and memory-safety for
processing large documentation sets.

Rather than building an MCP server from scratch, and rather than
reimplementing bundle *validation* from scratch, the core engine reuses
what [`jyjeanne/okf-rs`](https://github.com/jyjeanne/okf-rs) already gets
right — but **not** its bundle *writer*, which the Phase 0 spike
(`docs/dev/phase-0-findings.md`) found to be a poor fit:

| `okf-rs` crate | Status | Notes |
|---|---|---|
| `okf-core` | Reused | `okf.toml` config convention (§2.4) |
| `okf-validator` | Reused as-is | `validate_bundle(dir) -> ValidationReport` validates raw parsed frontmatter on disk — no dependency on the typed model below, confirmed working against a directly-written bundle (§6.4, §10) |
| `okf-mcp` | Pattern reused | JSON-RPC-over-stdio structure (§5.5); the code itself is DITA2Graph-specific |
| `okf-dita` | **Not reused** | `import_dita` is a generic, DITA-OT-independent XML→`Document` importer (no `keyref`/`conref`/DITAVAL resolution, no topic-type distinction) — not a substitute for §2's DITA-OT-driven extraction |
| `okf-generator` / `okf_parser::Concept` | **Not reused** | Hardcoded to a source-code vocabulary (`ConceptKind::{Package, Module, Class, Function, ...}`, `RelationKind::{Calls, Imports, ...}`) that cannot represent DITA topic types or the DITA relation taxonomy (§4.3) without upstream `okf-rs` changes |
| `okf-graph`, `okf-search` | Not reused (MVP) | Built on the same typed model as `okf-generator`; `dita2graph-core`'s own `graph.json` (§2.4) covers the MVP's query needs instead |

`dita2graph-core` writes its OKF bundle directly (`core/dita2graph-core/src/okf.rs`,
already implemented — see §12 Phase 0/1/2 status) rather than through
`okf-generator`. This is fully conformant: the OKF v0.2 *format* is just
markdown + YAML frontmatter with `type` as the only required key, nothing
about conformance requires going through `okf-rs`'s typed Rust API, and
this is verified empirically — the directly-written bundle passes
`okf_validator::validate_bundle` with zero errors (test in `okf.rs`).

DITA2Graph's own code is then mostly the DITA-specific extraction (§3.2)
and the DITA relation taxonomy (§4.3) layered on top — relation inference,
`conref`/`conkeyref` dedup, and audience/product/key mapping onto OKF
frontmatter (§4.1) — plus the DITA-specific MCP tools in §5.2, plus the
bundle writer itself.

**Phase 0's exit criterion (§12) is met** for the library-reuse half (all
of the crates above compile as git dependencies) but **not yet** for the
DITA-OT-preprocessing half — no live `dita --format dita2graph` run has
been attempted yet (`docs/dev/phase-0-findings.md`, finding 4). That
remains open at the top of the Phase 1 backlog.

### 3.1 Architecture

```
DITA-OT (Java)
       |
       | normalized DITA model (JSON over stdin/IPC)
       v
dita2graph-core (Rust)
       |
       | relation inference, conref/key dedup, OKF frontmatter mapping,
       | bundle writing (all dita2graph-specific -- src/model.rs, src/okf.rs)
       v
okf/ bundle (markdown + YAML frontmatter)
       |
       | okf_validator::validate_bundle (reused as-is, §6.4, §10)
       v
graph.json + dita2graph-mcp server (JSON-RPC-over-stdio, pattern from okf-mcp, §5.5)
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
  topic is `applies-to`". Low-confidence inferences are dropped rather than
  guessed (`DITA2GRAPH010W`, §2.5), so inferred edges never masquerade as
  author-declared ones.
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
  --store sqlite

dita2graph-core query \
  --store output/graph.db \
  --topic installing-product \
  --relation requires

dita2graph-core validate \
  --bundle output/okf   # runs okf-validator conformance checks (§6.4, §10)
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

> **Naming collision to watch for:** OKF's "concept" (any markdown
> document in a bundle) and DITA's `<concept>` topic type are different
> things that happen to share a word. A DITA `task` topic becomes an OKF
> *concept document* with `type: Task` — it does not become an OKF
> "Concept". See the glossary (§1.3) for the disambiguation convention
> used throughout this spec.

Only one frontmatter key is required — `type` — everything else
(`title`, `description`, `resource`, `tags`, provenance, trust, lifecycle)
is optional and consumers must tolerate unknown/missing keys gracefully
(OKF spec §11, "graceful degradation"). DITA2Graph's mapping from DITA
onto OKF frontmatter:

| DITA source | OKF frontmatter field |
|---|---|
| Topic type (`concept`/`task`/`reference`/`glossentry`) | `type` (e.g. `Concept`, `Task`, `Reference`, `Glossary Entry`) |
| `<title>` | `title` |
| `<shortdesc>`/`<abstract>` | `description` |
| Source `.dita` file path (relative, never absolute — §6.3) | `resource` |
| `audience`, `platform`, `product`, `otherprops` | `tags` |
| `conref`/`conkeyref` origin, resolved `keyref` | `sources` (provenance, OKF spec §5.1) |
| `dita2graph-core` version + generation timestamp | `generated: { by, at }` |
| Relation taxonomy edges (§4.3) not covered by a plain link | `relations` — a DITA2Graph **extension** field (spec explicitly allows producer-defined keys) |

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
fields — still fully conformant per OKF spec §11, since `type` is the
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
dita://ditaval/{name}
```

`dita://ditaval/{name}` exposes which DITAVAL profile a given bundle was
built with (audience/product/platform filters applied), so an agent can
tell whether it's looking at the public or an internal build before
trusting an answer's completeness (§6.1).

### 5.2 Tools

```
search_topics(query, topicType?, audience?, product?)
find_related_topics(topicId, relation?)
explain_task(topicId)
trace_dependencies(topicId, depth?)
generate_summary(topicId | mapId)
list_key_definitions(scope?)
validate_bundle()
```

`validate_bundle()` re-runs `okf-validator` conformance checks (§2.5,
§6.4, §10) on demand and returns pass/fail plus any violations — useful
for an agent (or a human) to confirm a bundle it's about to rely on is
actually well-formed before trusting its answers, without shelling out
to the CLI separately.

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

The plugin emits `mcp/mcp-server.toml`, generated from `cfg/dita2graph.xml`:

```toml
[server]
name = "dita2graph"
transport = "stdio"   # or "http" for remote/multi-client deployments (§6.3)

[graph]
store = "output/graph.db"
okf = "output/okf"

[resources]
enable = ["topics", "product", "architecture", "map", "relation", "ditaval"]

[tools]
enable = ["search_topics", "find_related_topics", "explain_task", "trace_dependencies", "generate_summary", "validate_bundle"]
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
        // explain_task, trace_dependencies, generate_summary, and
        // validate_bundle here, in the same shape (§5.2), dispatched to
        // dita2graph-core instead of okf-rs's generic okf-query layer.
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

## 6. Security and access control

Because DITA2Graph turns internal documentation into something an AI agent
can query directly and quote from, the trust boundary around the bundle
and the MCP server deserves explicit treatment — it wasn't addressed
elsewhere in this spec and is not optional for a real deployment.

### 6.1 Restricted content never enters the bundle

Audience/product/`otherprops` filtering must happen at DITAVAL-driven
extraction time (§2.2), not as a query-time access check on the MCP
server. Practically, this means maintaining (at minimum) two DITAVAL
profiles per product — `public.ditaval` and `internal.ditaval` — and
building two separate `okf/` bundles from them, rather than one bundle
with a "confidential" tag an agent is trusted to honor. A tag-based filter
bolted onto a single bundle is one prompt-injection or bug away from being
ignored; a topic that was never extracted cannot leak.

### 6.2 Local (stdio) transport is the safe default

The default `transport = "stdio"` (§5.4) confines the MCP server to the
same trust boundary as running any other local CLI tool the user already
has filesystem access to — no additional authentication is required for
this mode, since there's no network exposure to authenticate against.

### 6.3 HTTP transport requires authentication (not yet implemented)

The planned HTTP transport (§13) multiplies exposure — a remote,
multi-client MCP server — and must not ship without:

- An API key or OAuth-based auth layer in front of `dita2graph-mcp`.
- Per-audience scoping that mirrors DITA's own `audience` attribute, so
  a caller authenticated as "customer support" cannot query a bundle
  built from `internal.ditaval`.
- Transport-level TLS; the stdio pattern in §5.5 has no encryption story
  of its own and must not be naively reused verbatim over a raw socket.

Until this lands, the MCP server should be treated as **local-only** in
any deployment guidance.

### 6.4 No secrets or unintended provenance leakage in the bundle

- The core engine must not copy API keys, credentials, or internal
  hostnames from DITA source metadata (`<data>`, `othermeta`) into OKF
  frontmatter or body content.
- **Implemented and shipped** (§12 Phase 5): `okf-validator` is an
  external dependency (§3) and isn't ours to extend with a
  DITA2Graph-specific rule, so the check lives in `dita2graph-core`
  itself — `core/dita2graph-core/src/secrets.rs`'s `scan_bundle()` walks
  every file in the written bundle for high-confidence secret shapes
  (AWS access key IDs, PEM private-key headers, GitHub and Slack token
  prefixes) and is run from `validate_and_report()` in `main.rs`
  immediately after the `okf-validator` conformance check, for both
  `build` and `validate`. A match is a build-breaking error, not a
  warning: `DITA2GRAPH050E` is emitted and the process exits `1`, exactly
  like a failed `okf-validator` check (§2.5). Deliberately narrow — no
  generic `password=`/`api_key=` heuristic, since that would false-positive
  on ordinary documentation prose (a topic titled "How to reset your
  password" is not a leak). Covered by 7 unit tests plus a manual
  end-to-end smoke test (`build` against a normalized model containing a
  planted AWS key correctly fails with exit code 1 and the diagnostic; the
  same model with the key removed passes).
- `resource`/`sources` paths in generated frontmatter are always
  bundle-relative (`topics/installing-product.dita`), never absolute
  filesystem paths — absolute paths on a build machine can leak usernames,
  internal directory structure, or CI system layout into a bundle that
  might be published publicly (§4.1 table, §14).

### 6.5 Supply-chain note on reused crates

Since the core engine depends on `okf-rs` crates (§3) rather than
vendoring their logic, DITA2Graph inherits their dependency tree and
release cadence. Pin exact `okf-rs` crate versions (not a floating range)
in `Cargo.toml`, and re-run the validation suite (§10) on every `okf-rs`
version bump before adopting it, the same discipline applied to the
DITA-OT/OKF baseline in §1.1.

---

## 7. Recommended implementation stack

| Component | Technology |
|---|---|
| DITA processing | DITA-OT 4.4 |
| DITA standard | DITA 1.3 |
| Plugin (DITA-OT integration) | Java 25 (LTS) / Ant / Gradle 9.0+ |
| Build automation | Gradle 9.0 minimum, currently building against 9.6.x, Kotlin DSL (`build.gradle.kts`) preferred over Groovy (via `dita-ot-gradle`, see section 8) |
| Graph engine | Rust (latest stable, currently 1.97.1, edition 2024) — own OKF bundle writer; reuses `okf-core` (config) and `okf-validator` (validation) from `okf-rs` as-is, not `okf-dita`/`okf-generator` (§3) |
| Knowledge format | OKF v0.2 |
| Agent interface | MCP (JSON-RPC 2.0, protocol rev. 2024-11-05), pattern from `okf-mcp` |
| Storage | SQLite / RocksDB (derived index only — the bundle itself is markdown; not yet implemented, §12 Phase 2 status) |
| CLI | Rust (Clap) |
| Serialization | Markdown + YAML frontmatter (bundle) / JSON (derived `graph.json`) |
| MCP transport | stdio (local, default) / HTTP (remote, planned — requires auth, §6.3) |

---

## 8. Build integration with Gradle (`dita-ot-gradle`)

For teams that already build their DITA output with Gradle rather than
invoking `dita` directly, DITA2Graph is designed to slot into
[`jyjeanne/dita-ot-gradle`](https://github.com/jyjeanne/dita-ot-gradle) — a
Gradle plugin (`io.github.jyjeanne.dita-ot-gradle`) that manages DITA-OT
itself: downloading it, installing plugins into it (including
`org.dita.dita2graph`), validating content, and running transformations,
all as ordinary Gradle tasks with up-to-date checking and configuration
cache support.

**Gradle 9.0 is a hard minimum** (§1.1), not just a recommendation: that's
where `dita-ot-gradle` itself reaches full support, and it's the version
this spec's examples are written and tested against (currently 9.6.x).
Pin it via the Gradle wrapper (`./gradlew wrapper --gradle-version 9.6.1`)
rather than relying on whatever Gradle happens to be on a machine's `PATH`.

### 8.1 Why use it

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

### 8.2 Example `build.gradle.kts` (Kotlin DSL)

Pinned to DITA-OT **4.4** and Java **25**, matching the baseline in §1.1
(requires Gradle 9.0+, §8). Kotlin DSL is the primary example here —
it's what `dita-ot-gradle` itself is written in (it ships a
`gradle-kotlin-dsl` topic), gets IDE autocomplete/type-checking that the
Groovy DSL doesn't, and is what Gradle itself now recommends for new
builds. A Groovy DSL equivalent follows for teams standardized on it.

```kotlin
import com.github.jyjeanne.DitaOtDownloadTask
import com.github.jyjeanne.DitaOtInstallPluginTask
import com.github.jyjeanne.DitaOtValidateTask
import com.github.jyjeanne.DitaLinkCheckTask
import com.github.jyjeanne.DitaOtTask

plugins {
    id("io.github.jyjeanne.dita-ot-gradle") version "2.8.6"
}

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(25))
    }
}

val downloadDitaOt = tasks.register<DitaOtDownloadTask>("downloadDitaOt") {
    version("4.4")
}

val installDita2Graph = tasks.register<DitaOtInstallPluginTask>("installDita2Graph") {
    dependsOn(downloadDitaOt)
    ditaOtDir(layout.buildDirectory.dir("dita-ot/dita-ot-4.4"))
    plugins("org.dita.dita2graph")   // local path, URL, or registry id
    force.set(false)
}

// DitaOtValidateTask uses ditaOtDir(...), not ditaOt(...) -- that name
// only exists on DitaOtTask (§12 Phase 0/1 status). Depends on
// downloadDitaOt directly, not installDita2Graph: validation doesn't
// need our plugin installed, and decoupling means it (and checkLinks)
// still run -- and still gate buildKnowledgeGraph below -- even on a
// build where plugin installation itself fails.
val validateDocs = tasks.register<DitaOtValidateTask>("validateDocs") {
    dependsOn(downloadDitaOt)
    ditaOtDir(layout.buildDirectory.dir("dita-ot/dita-ot-4.4"))
    input("docs/user-guide.ditamap")
}

// DitaLinkCheckTask has no ditaOtDir/DITA-OT dependency at all -- it's a
// pure Kotlin XML link scanner (confirmed from its source), so it needs
// neither dependsOn(downloadDitaOt) nor a ditaOtDir(...) call.
val checkLinks = tasks.register<DitaLinkCheckTask>("checkLinks") {
    input("docs/user-guide.ditamap")
}

val buildKnowledgeGraph = tasks.register<DitaOtTask>("buildKnowledgeGraph") {
    dependsOn(installDita2Graph, validateDocs, checkLinks)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-4.4"))
    input("docs/user-guide.ditamap")
    output("build/dita2graph")
    transtype("dita2graph")

    properties {
        "args.dita2graph.mcp" to "true"
        "args.dita2graph.store" to "sqlite"
        "args.dita2graph.depth" to "3"
    }

    progressStyle("DETAILED")
}

// Normal HTML5 publishing can run alongside graph generation
val publishHtml = tasks.register<DitaOtTask>("publishHtml") {
    dependsOn(installDita2Graph, validateDocs, checkLinks)
    ditaOt(layout.buildDirectory.dir("dita-ot/dita-ot-4.4"))
    input("docs/user-guide.ditamap")
    output("build/docs/html")
    transtype("html5")
}

tasks.register("publishAll") {
    dependsOn(buildKnowledgeGraph, publishHtml)
}
```

Run it (identical either way — the DSL choice only affects the build
script, not the CLI):

```bash
./gradlew buildKnowledgeGraph
# or, to validate, publish HTML, and build the graph together
./gradlew publishAll
```

> **Verified**, not guessed: this exact example (adapted to this repo's
> own paths) was run against a live Gradle 9.6.1 + DITA-OT 4.4 in
> `gradle-build/` (`docs/dev/phase-0-findings.md` findings 5–7).
> `./gradlew buildKnowledgeGraph` now runs the **entire pipeline for
> real** — download DITA-OT, install the plugin, validate, check links,
> run the `dita2graph` transtype — and produces a real,
> `okf_validator`-passing OKF bundle, now that `lib/dita2graph-core.jar`
> is a real, built artifact (§12 Phase 1 status) rather than a
> placeholder. The property-setter calls above (`ditaOtDir(...)`,
> `plugins(...)`, `force.set(...)`, `ditaOt(...)`, `input(...)`,
> `transtype(...)`, `progressStyle(...)`) were corrected against
> `dita-ot-gradle`'s actual Kotlin source after the original version of
> this example (using `plugins(listOf(...))`, `force(false)`, and
> `ditaOt(...)` on `DitaOtValidateTask`) failed to compile — see finding
> 5 for exactly what was wrong and why. `tasks.register<Type>("name")
> { }` is used throughout rather than `by tasks.registering(Type::class)`,
> which Gradle 9.6 deprecates as incompatible with Gradle 10.
>
> **Local-development note:** the `plugins("org.dita.dita2graph")` call
> above assumes installing from the plugin registry (once published) or
> a pre-built ZIP — that's what `DitaOtInstallPluginTask`'s "local" path
> actually expects. Pointing it straight at a plugin *source directory*
> (e.g. this repo's own `plugin/org.dita.dita2graph/` while developing)
> fails with "Failed to expand ... to .../plugin", the same error
> `java.util.zip` gives reading a directory as a zip stream — confirmed
> the hard way (finding 7). `gradle-build/build.gradle.kts` shows the
> fix: a `Zip` task ahead of `installDita2Graph`, pointing `plugins(...)`
> at the zip's output file instead of the raw directory.

#### Groovy DSL equivalent (`build.gradle`)

Not independently re-run in `gradle-build/` (only the Kotlin DSL version
above was) — Groovy's dynamic property assignment tends to work against
Gradle's `Property<T>`/`ListProperty<T>` types in more cases than
Kotlin's static typing allows without `.set(...)`, but treat this as
translated-and-plausible rather than verified the same way the Kotlin
version is.

```groovy
plugins {
    id 'io.github.jyjeanne.dita-ot-gradle' version '2.8.6'
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(25)
    }
}

tasks.register('downloadDitaOt', com.github.jyjeanne.DitaOtDownloadTask) {
    version = '4.4'
}

tasks.register('installDita2Graph', com.github.jyjeanne.DitaOtInstallPluginTask) {
    dependsOn downloadDitaOt
    ditaOtDir = layout.buildDirectory.dir('dita-ot/dita-ot-4.4')
    plugins = ['org.dita.dita2graph']   // local path, URL, or registry id
    force = false
}

tasks.register('validateDocs', com.github.jyjeanne.DitaOtValidateTask) {
    dependsOn downloadDitaOt
    ditaOtDir layout.buildDirectory.dir('dita-ot/dita-ot-4.4')
    input 'docs/user-guide.ditamap'
}

// DitaLinkCheckTask has no ditaOtDir/DITA-OT dependency at all -- a pure
// Kotlin XML link scanner, same as the Kotlin DSL version above.
tasks.register('checkLinks', com.github.jyjeanne.DitaLinkCheckTask) {
    input 'docs/user-guide.ditamap'
}

tasks.register('buildKnowledgeGraph', com.github.jyjeanne.DitaOtTask) {
    dependsOn installDita2Graph, validateDocs, checkLinks
    ditaOt layout.buildDirectory.dir('dita-ot/dita-ot-4.4')
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
    dependsOn installDita2Graph, validateDocs, checkLinks
    ditaOt layout.buildDirectory.dir('dita-ot/dita-ot-4.4')
    input 'docs/user-guide.ditamap'
    output 'build/docs/html'
    transtype 'html5'
}

tasks.register('publishAll') {
    dependsOn buildKnowledgeGraph, publishHtml
}
```

### 8.3 CI recommendation

```properties
# gradle.properties
org.gradle.configuration-cache=true
org.gradle.parallel=true
```

Use `progressStyle = 'QUIET'` on `buildKnowledgeGraph` in CI so logs stay
readable, and gate `buildKnowledgeGraph` behind `validateDocs` and
`checkLinks` succeeding, so a broken source map never produces a stale or
partially-built graph that an AI agent could query with confidence it
doesn't deserve. See §10 for what else should run in this pipeline
(bundle conformance, MCP protocol tests, regression corpus).

---

## 9. Why this architecture is interesting

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

### 9.1 Comparison with standard RAG

| Dimension | Standard RAG (text → chunks → embeddings) | DITA2Graph (DITA → OKF → MCP) |
|---|---|---|
| Unit of retrieval | Arbitrary N-token text chunk, often splitting mid-thought | A whole DITA concept (topic, one coherent unit the author already scoped) |
| Relationships | Inferred at query time from embedding similarity | Explicit, author-declared (`reltable`, `keyref`, `xref`) plus a typed taxonomy (§4.3) |
| Reuse (conref) | Duplicated across every chunk that includes it, inflating the index and risking drift between copies | Single canonical concept, referenced by `sources`/relations — one source of truth |
| Applicability (audience/product/version) | Usually lost, or hacked in as metadata filters bolted onto the vector store | First-class (`tags`, DITAVAL-driven), enforced at extraction time (§6.1) |
| Answer for "what does X require?" | Best-effort: retrieve top-k similar chunks, hope one mentions a dependency | Deterministic: one graph traversal (`requires` edge) |
| Freshness / staleness detection | Re-embed everything, or nothing | Incremental rebuild keyed by source file hash (§3.3); `generated.at` frontmatter makes staleness inspectable |
| Failure mode on bad input | Silent — a broken doc still embeds and gets retrieved | Loud — `okf-validator` and DITA-OT's own key/conref resolution reject broken source before a graph is built (§9.3) |
| Output artifact | Vectors in a proprietary store; not human-readable | Plain markdown bundle; human-readable, diffable, greppable without tooling |
| Access control | Usually none, or bolted on after the fact | Enforced at extraction (§6.1) via separate DITAVAL-built bundles |

### 9.2 Reduced token consumption

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
search plus several speculative chunk reads. This is a directional claim,
not a guaranteed number — §10 calls for measuring it against a real
regression corpus rather than citing `okf-rs`'s figure as DITA2Graph's own.

### 9.3 Validation

Because the graph is built from DITA-OT's own preprocessing pipeline
rather than scraped from rendered output, DITA2Graph gets validation for
free at multiple layers before an agent ever sees the data:

- **DITA-OT's own reference resolution fails the build on a broken
  `href`**, confirmed directly (`docs/dev/phase-0-findings.md` finding
  8): an `<xref href="missing.dita"/>` to a nonexistent file is a hard
  `[DOTX008E]` error and a non-zero exit. An unresolved **`keyref`** is
  *not* equally strict, though — DITA-OT treats it as informational
  (`[DOTJ047I] ... using the @href attribute as fallback if it exists`)
  and the build succeeds with the target silently dropped, useful for
  authoring against a partial key space but a real gap in this
  guarantee: a `keyref`-only broken reference can make it into the
  graph the way a broken plain link cannot.
- **`dita-ot-gradle`'s `DitaOtValidateTask`/`DitaLinkCheckTask`** (§8)
  catch XML validity, reference integrity, and dead links *before*
  `dita2graph` runs, gating graph generation on green validation — this
  is where the CI-observed failure in §12 Phase 4's exit criterion
  actually comes from, and it's likewise strict on broken `href`s, not
  (currently) on unresolved `keyref`s.
- **`okf-validator`** checks the emitted bundle for OKF v0.2 schema
  conformance and link integrity (every `relations`/markdown-link target
  actually resolves to a concept in the bundle) — the graph can't claim a
  relationship to a concept that doesn't exist.
- **Typed edges are structurally checked**, not inferred: a `requires`
  edge exists only if the source DITA actually declared that
  relationship, so there's no equivalent of an embedding model
  hallucinating a similarity that isn't really there.

### 9.4 Native AI-tool interaction

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

## 10. Testing strategy

None of the guarantees claimed in §9 hold unless they're tested, so
testing is treated as a first-class part of the spec rather than an
afterthought:

- **Unit tests (`dita2graph-core`, Rust)**: `cargo test` around
  normalization, relation inference, and `conref`/`conkeyref`
  deduplication (§3.3); golden-file tests comparing generated OKF
  concepts against checked-in fixtures for a representative multi-topic
  sample project.
- **Plugin integration tests (Java/Ant side)**: leverage DITA-OT's own
  plugin test framework (`test/testinput` + `test/testexpected`
  fixtures, run via the toolkit's `ant test`) rather than inventing a
  parallel harness, so DITA2Graph's tests run the same way every other
  DITA-OT plugin's tests do.
- **Bundle conformance tests**: run `okf-validator` against every
  generated bundle as a required CI gate (§2.5, §6.4, §9.3) — a bundle
  that fails conformance fails the build, full stop.
- **MCP protocol tests**: JSON-RPC round-trip tests for `initialize`,
  `tools/list`, and `tools/call`, following the pattern already present
  in `okf-mcp`'s own `#[cfg(test)] mod tests` (§5.5) — reuse that
  structure for `dita2graph-mcp`'s DITA-specific tools.
- **End-to-end tests**: a small but representative reference project
  (multiple topic types, at least two audience/product DITAVAL profiles,
  at least one `conref` and one `conkeyref`) exercised through the full
  pipeline — `dita --format dita2graph` → bundle → `dita2graph-mcp query`
  — in CI, as part of the Gradle build (§8).
- **Regression corpus**: that reference project is maintained as a fixed,
  version-controlled fixture so future changes to relation inference or
  OKF mapping have something concrete to regress against; it also
  doubles as the basis for measuring the token-consumption claim in
  §9.2 empirically instead of by analogy to `okf-rs`.
- **Security tests**: a fixture pair (`public.ditaval`/`internal.ditaval`
  builds of the same source, §6.1) with an automated check that no
  internal-only topic ID appears in the public bundle's `okf/` tree.

---

## 11. MVP scope

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
5. A `build.gradle.kts` (Kotlin DSL) example (section 8) wiring `dita-ot-gradle` to
   download DITA-OT 4.4, install the plugin, validate content
   (`okf-validator` + `DitaOtValidateTask`/`DitaLinkCheckTask`), and run
   the `dita2graph` transtype as part of a normal CI build.
6. The public/internal DITAVAL split (§6.1) and the regression corpus
   (§10), so the MVP demonstrates the access-control and validation
   guarantees this spec claims, not just the happy path.

That scope is already a self-contained, demonstrable, open-source project:
DITA in, queryable knowledge graph and live MCP server out. Section 12
breaks this same scope into ordered, gated delivery phases.

---

## 12. Development phases

Each phase has a goal, concrete deliverables, and an exit criterion that
gates moving to the next phase — a phase isn't "done" on a time box, it's
done when its exit criterion is met.

### Phase overview

| Phase | Focus | Rough duration | Depends on |
|---|---|---|---|
| 0 | Spike & feasibility | 1–2 weeks | — |
| 1 | Plugin skeleton + normalized model | 2–3 weeks | Phase 0 |
| 2 | Core engine: OKF bundle generation | 3–4 weeks | Phase 1 |
| 3 | MCP server | 2–3 weeks | Phase 2 |
| 4 | Gradle integration + CI hardening | 2 weeks | Phase 3 |
| 5 | Security, docs, public v0.1.0 release | 2 weeks | Phase 4 |
| 6+ | Extended capabilities (post-MVP) | Ongoing, per-item | Phase 5 |

Total to a public v0.1.0: roughly **12–16 weeks** for a small team (1–2
engineers), assuming the Phase 0 reuse assumption (§3) holds.

### Phase 0 — Spike & feasibility (1–2 weeks)

**Goal:** de-risk the two biggest unknowns before committing to the
architecture — that DITA-OT's preprocessing exposes everything §2.2/§3.2
assume, and that `okf-rs` crates are usable as library dependencies (§3)
rather than CLI-only.

**Deliverables:**
- A throwaway script that dumps DITA-OT's resolved intermediate XML for
  one sample ditamap and confirms resolved `keyref`/`conref`/DITAVAL
  filtering are all present at the point DITA2Graph would hook in.
- A spike Rust program that hand-writes one OKF concept file and passes
  it through `okf-validator` as a library call (not the `okf-rs` CLI).

**Exit criteria:** both confirmed working, or a documented fallback
decision (§3's "verifying the reuse assumption" note) if `okf-dita`/
`okf-generator` turn out not to be usable as libraries.

**Status:** done, both halves. `okf-core`/`okf-validator` confirmed
reusable; `okf-dita`/`okf-generator` confirmed *not* reusable for the
core write path (fallback decision taken: `dita2graph-core` writes its
own bundle, §3). The DITA-OT-preprocessing half is now also confirmed:
DITA-OT 4.4 was downloaded and run for real against `sample-docs/`
(`gradle-build/`, `docs/dev/phase-0-findings.md` finding 4/5) — its
resolved intermediate topic for `installing-product.dita` shows the
`keyref="config-concept"` cross-reference resolved to a concrete
`href="configuration.dita"`, with `audience`/`product` profiling
preserved, exactly matching §3.2's assumptions.

### Phase 1 — DITA-OT plugin skeleton + normalized model (2–3 weeks)

**Goal:** `org.dita.dita2graph` installs cleanly and runs end-to-end on a
sample map, producing the normalized DITA model (§3.2) — no OKF or MCP
yet.

**Deliverables:** `plugin.xml`, `build.xml`, `cfg/dita2graph.xml`,
`cfg/messages.xml` (§2.1, §2.5); `bin/dita2graph` CLI stub; one working
`dita --format dita2graph` invocation against a 3-topic sample map.

**Exit criteria:** plugin installs via `dita --install` with no manual
steps; the normalized model JSON for the sample map validates against a
JSON Schema for §3.2 and correctly reflects resolved keys, `conref`, and
one DITAVAL-filtered topic.

**Status:** done, in substance. `plugin/org.dita.dita2graph/{plugin.xml,
build.xml, cfg/dita2graph.xml, cfg/messages.xml, bin/dita2graph,
lib/dita2graph-core.jar}` and `sample-docs/` (a 3-topic ditamap with
`keyref`, one cross-reference, and `audience`/`product` profiling) exist,
and the whole pipeline is **verified end-to-end against a live DITA-OT
4.4 install** (`docs/dev/phase-0-findings.md` findings 5–7): `dita
--format dita2graph --input sample-docs/user-guide.ditamap` and
`./gradlew buildKnowledgeGraph` (in `gradle-build/`) both install the
plugin, run DITA-OT's real preprocessing, execute
`org.dita.dita2graph.tasks.ExtractTask` (built by
`plugin/org.dita.dita2graph/java/`), and produce a real,
`okf_validator`-passing OKF bundle. Findings 5–7 caught and fixed seven
real bugs the spec/harness had gotten wrong along the way: illegal `--`
in XML comments, two nonexistent extension points, the `dita2` +
transtype-name Ant target-naming convention, the
`build-init,preprocess2` dependency chain DITA-OT 4.4 needs, missing
`args.dita2graph.*` property defaults, the wrong output-directory
property name, and `dita-ot-gradle`'s local-plugin-install needing a
ZIP rather than a raw directory.

Extraction itself is intentionally narrow, not a shortcut taken
silently: `contains` (map `<topicref>`), `requires` and `references`
(`<xref>`/`<link>`, gated on `keyref`) are the only relations derived,
leaving `applies-to`/`related-to`/`generated-from` as documented future
inference work (§3.3, §13) rather than guessed. Nested
`topicref`/`topichead`/`topicgroup` map structures aren't walked either
(direct `<topicref>` children only) — a real gap for maps deeper than
`sample-docs/`'s flat one. The exit criterion's "validates against a
JSON Schema" is met in spirit but not literally: there's no standalone
`.schema.json` file, only the Rust side's `serde` struct definitions
acting as the de facto (and enforced) schema — worth formalizing if a
second producer of the normalized model ever appears.

### Phase 2 — Core engine: OKF bundle generation (3–4 weeks)

**Goal:** `dita2graph-core` consumes the normalized model, performs
relation inference and `conref`/`conkeyref` dedup (§3.3), and writes a
conformant OKF v0.2 bundle.

**Deliverables:** `okf/` bundle output for the sample map (§4.4); derived
`graph.json`; incremental rebuild (source-hash keyed, §3.3) working on a
second run.

**Exit criteria:** generated bundle matches a checked-in golden fixture
byte-for-byte (modulo timestamps); 100% `okf-validator` pass; a no-op
re-run touches zero unchanged concept files.

**Status:** mostly done, ahead of Phase 1. `core/dita2graph-core`
implements the normalized model (`src/model.rs`), the bundle writer
(`src/okf.rs`), the `DITA2GRAPHnnnX` diagnostics catalog
(`src/diagnostics.rs`), and a `build`/`validate`/`query` CLI
(`src/main.rs`); `cargo test -p dita2graph-core` passes, including a test
that builds a bundle and round-trips it through `okf_validator::
validate_bundle` with zero errors. **Not done:** relation *inference*
(§3.3's "topics sharing a `product` value are `related-to`" kind of
derived edge — only author-declared relations from the normalized model
are handled so far), incremental rebuild, and SQLite/RocksDB storage
(`query` currently reads `graph.json` directly, not a database). No
golden-fixture byte-for-byte test yet either. This phase got ahead of
Phase 1 because it could be developed and tested against a hand-authored
fixture without needing a live DITA-OT install — closing Phase 1's gap
may still change `dita2graph-core`'s input shape at the edges.

### Phase 3 — MCP server (2–3 weeks)

**Goal:** `dita2graph-mcp` implements the JSON-RPC-over-stdio pattern
(§5.5) with the DITA-specific tool set (§5.2), including `validate_bundle`.

**Deliverables:** `dita2graph-mcp` binary; `mcp-server.toml` emission
(§5.4); passing protocol tests (§10) for `initialize`/`tools/list`/
`tools/call`.

**Exit criteria:** a live Claude Code session, registered against the
sample bundle's server, correctly answers the §5.3 example interaction
end to end (`search_topics` → related tasks), with tool calls visibly
returning small, typed results rather than raw file contents.

**Status:** mostly done. `mcp/dita2graph-mcp` implements the pattern from
§5.5 with `search_topics`/`find_related_topics`/`explain_task`/
`trace_dependencies`/`generate_summary`/`validate_bundle`; protocol and
tool tests pass (`cargo test -p dita2graph-mcp`), and it was exercised
manually end-to-end over real stdin/stdout JSON-RPC against the
`sample-docs/` bundle, correctly answering a `search_topics`/
`explain_task` sequence. **Not done:** the exit criterion specifically
asks for a live Claude Code session registered against it, which hasn't
been done in this session; `mcp-server.toml` emission (§5.4) also isn't
wired up yet — the server currently takes the bundle root as a plain CLI
argument.

### Phase 4 — Gradle integration + CI hardening (2 weeks)

**Goal:** wire the `dita-ot-gradle` tasks (§8) end-to-end and turn the
testing strategy (§10) into an enforced CI pipeline.

**Deliverables:** the reference `build.gradle.kts` (Kotlin DSL, §8.2);
green CI building the sample project on every push; the regression
corpus and public/internal DITAVAL security test (§10) both wired in as
required checks.

**Exit criteria:** a deliberately introduced broken `conref` in the
sample project fails CI at `validateDocs`/`checkLinks`, before
`buildKnowledgeGraph` ever runs — reproducing §9.3's guarantee as an
actual, observed CI failure rather than a claim.

**Status:** exit criterion met, though with a broken *reference* rather
than specifically a broken `conref` — `sample-docs-invalid/` (a
`<xref href="does-not-exist.dita"/>` to a nonexistent file) and
`gradle-build/`'s `validateBrokenDoc` task reproduce the guarantee as an
observed CI failure (`.github/workflows/integration.yml`), not a claim.
Worth noting since it wasn't obvious going in: an unresolvable `keyref`
does **not** trigger this — DITA-OT treats it as informational
(`[DOTJ047I] ... Using the @href attribute as fallback if it exists`)
and the build succeeds anyway, confirmed by trying it first and finding
that out the hard way. Only an unresolvable `href` (or similar hard
reference-integrity break) produces the `[DOTX008E]`-class error
`DitaOtValidateTask` actually fails on. `./gradlew buildKnowledgeGraph`
itself (the "green CI building the sample project" deliverable) is also
in CI now, alongside the Rust and Java unit-test workflows. **Not yet
done:** the rest of §10's testing strategy (plugin-integration tests via
DITA-OT's own test framework, a broader regression corpus beyond the one
small fixture, MCP protocol tests running against a *live* `dita2graph`-
built bundle rather than a hand-built one in `dita2graph-mcp`'s own unit
tests) — CI today only covers what's described above.

### Phase 5 — Security, docs, and public v0.1.0 release (2 weeks)

**Goal:** apply §6 end-to-end and make the project usable by someone who
isn't the person who built it.

**Deliverables:** public/internal DITAVAL split shipped as a documented
pattern (§6.1); secret-leakage detection rule (§6.4); a user-facing
README/quickstart (§15); `LICENSE` file (§14); tagged `v0.1.0` matching
the MVP scope in §11.

**Exit criteria:** an external tester, given only the README and no other
context, gets from a DITA project to a working `dita2graph-mcp` server
answering queries in Claude Code, hitting no undocumented step.

**Status:**
- Done — licensing decided and shipped: dual **MIT OR Apache-2.0** applied
  uniformly across every component (Rust, Java, Gradle); `LICENSE`,
  `LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE` (attribution for the
  `okf-mcp`-derived transport code and the `okf-rs`/`dita-ot-gradle`
  dependencies) added at the repo root; `Cargo.toml`'s
  `[workspace.package].license` matches (§14).
- Done — secret-leakage detection (§6.4): implemented in
  `core/dita2graph-core/src/secrets.rs`, wired into `validate_and_report`
  in `main.rs` so both `build` and `validate` fail (`DITA2GRAPH050E`,
  exit code 1) on a detected secret. Verified with unit tests and a
  manual CLI smoke test against a planted AWS key.
- Not started — public/internal DITAVAL split is not yet a *demonstrated*
  pattern: `sample-docs/public.ditaval` currently has nothing to filter,
  since no fixture topic is marked `audience="internal"`. Needs a fixture
  update and a build comparison (public vs. internal output actually
  differs) before this deliverable is real rather than aspirational.
- Not started — README/quickstart polish toward the exit criterion above,
  and the `v0.1.0` tag itself (deliberately held for explicit user
  confirmation before tagging/releasing, per this project's own practice
  of treating visible, hard-to-reverse actions as confirm-first).

### Phase 6+ — Extended capabilities (post-MVP, ongoing)

Not a single phase but a backlog, picked up item-by-item based on
adopter feedback after v0.1.0 — see §13 for the current list (multi-map
federation, graph diffing, HTTP transport + auth, embedding-based
`related-to` inference, a rendered-output annotation variant). Each item
gets its own scoped follow-up spec/issue and its own exit criterion
before work starts, rather than being bundled into one open-ended phase.

---

## 13. Future work

- Relation inference beyond explicit DITA markup (e.g. embedding-based
  `related-to` suggestions layered on top of the structural graph, clearly
  distinguished from author-declared relationships).
- Multi-map / multi-product graph federation (merging graphs from several
  `ditamap`s into one queryable knowledge base, e.g. per-product graphs
  joined into an org-wide graph).
- Graph versioning/diffing across doc releases, so an agent can answer
  "what changed about authentication between v3 and v4?".
- HTTP transport for the MCP server to support shared/remote deployments,
  with authentication and per-audience access control mirroring DITA's
  `audience` filtering (§6.3).
- A DITA-OT PDF/HTML5 plugin variant that annotates rendered output with
  links back into the graph (e.g. "view related topics" panels driven by
  `dita2graph.find_related_topics`).

---

## 14. Licensing

**Decided and shipped** (§12 Phase 5): dual **MIT OR Apache-2.0**,
applied uniformly across every component in the repository — the Rust
workspace, the DITA-OT plugin (Java/Ant/XML), and the Gradle integration
harness — via root `LICENSE`/`LICENSE-MIT`/`LICENSE-APACHE` files, plus
a `NOTICE` file for third-party attribution. This supersedes an earlier
draft of this section that sketched *different* licenses per component
(Apache-2.0 for the Java/Ant side specifically, to match `dita-ot-gradle`
and DITA-OT; dual MIT/Apache-2.0 for Rust): one consistent license across
a single repository is simpler for downstream users to reason about, and
is a strict superset of what Apache-2.0-only would permit — anyone who
specifically wants Apache-2.0 terms (e.g. for DITA-OT-ecosystem
compatibility) still gets to choose them.

- **Reused `okf-rs` source**: the JSON-RPC-over-stdio transport in
  `mcp/dita2graph-mcp/src/main.rs` is adapted from `jyjeanne/okf-rs`'s
  `okf-mcp` crate (itself dual MIT/Apache-2.0, §3, §5.5) — attributed in
  both that file's own doc comment and in `NOTICE`, not just a
  spec-level citation.
- **This specification document**: still unlicensed for standalone
  redistribution outside the code repository; a permissive documentation
  license (e.g. CC-BY-4.0) remains a good choice if that's ever needed,
  but hasn't been applied since the document has only been published as
  part of this repository so far.

---

## 15. Appendix A: Quickstart

A minimal path from a DITA project to a queryable MCP server, assuming
the MVP scope (§11) is implemented:

```bash
# 1. Install DITA-OT 4.4 and the DITA2Graph plugin (§2)
dita --install org.dita.dita2graph

# 2. (Optional) Build with Gradle instead of the raw CLI (§8)
./gradlew buildKnowledgeGraph

# 3. Or invoke DITA-OT directly (§2.3)
dita \
  --input user-guide.ditamap \
  --format dita2graph \
  --filter public.ditaval \
  --args.dita2graph.mcp=true

# 4. Inspect the generated bundle (§4) — it's just markdown
ls output/okf/topics/
cat output/okf/topics/installing-product.md

# 5. Validate it explicitly (§2.5, §6.4, §10)
dita2graph-core validate --bundle output/okf

# 6. Start the MCP server (§5.4)
dita2graph-mcp serve --config output/mcp/mcp-server.toml

# 7. Register it with Claude Code
claude mcp add dita2graph -- dita2graph-mcp serve --config output/mcp/mcp-server.toml
```

From here, asking Claude Code a documentation question (§5.3) should
route through `search_topics`/`find_related_topics` rather than a raw
file read — that's the signal the integration is working end to end.
