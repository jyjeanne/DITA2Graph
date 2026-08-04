# Architecture & Diagrams

This document collects the UML-style diagrams for DITA2Graph, each
using the diagram type best suited to what it's showing:

- **[Component diagram](#component-diagram--system-architecture)** — the
  overall system architecture: how the DITA-OT plugin, the Rust core
  engine, the generated bundle, the MCP server, and AI agents fit
  together as deployable pieces.
- **[Class diagrams](#class-diagrams--rust-architecture)** — the Rust
  type architecture: the normalized model in `core/dita2graph-core`
  and the in-memory bundle index in `mcp/dita2graph-mcp`.
- **[Activity diagrams](#activity-diagrams--internal-processing-workflows)**
  — internal processing workflows: DITA parsing/extraction, OKF/RAG
  graph generation, and the (planned, not yet implemented) incremental
  update flow.
- **[Sequence diagram](#sequence-diagram--ai-agent--mcp-server--knowledge-graph)**
  — a concrete interaction between an AI agent, the MCP server, and the
  knowledge graph on disk.

All diagrams are Mermaid and render natively in GitHub. They're kept
here, separate from the top-level `README.md`'s flowchart, because
they go one level deeper — into module boundaries, types, and method
signatures — which belongs in developer documentation rather than the
project's front page. Every diagram is derived directly from the code
(`core/dita2graph-core/src/`, `mcp/dita2graph-mcp/src/`) and the design
spec (`docs/plugin-specification.md`), not from an idealized version of
either; where a diagram shows something not yet implemented, it says so
explicitly, in keeping with this repo's `README.md`/`Roadmap.md`
practice of documenting real gaps rather than hiding them.

## Component diagram — system architecture

How the pieces are deployed and what crosses which boundary: the DITA
repository feeds DITA-OT, DITA-OT dispatches to the `org.dita.dita2graph`
plugin, the plugin's Java `ExtractTask` shells out to the Rust
`dita2graph-core` binary, which writes the OKF bundle and RAG index to
the filesystem, which `dita2graph-mcp` reads back and serves to AI
agents over JSON-RPC. `gradle-build/` and CI are dev/build-time
harnesses, not part of the runtime path.

```mermaid
flowchart TB
    subgraph SRC["DITA Source"]
        Repo["DITA Repository<br/>ditamap · topics · keys · conrefs · DITAVAL"]
    end

    subgraph OT["DITA-OT Pipeline (external toolkit)"]
        Preprocess["preprocess2 / build-init<br/>resolves keys, conrefs, topicref hierarchy,<br/>applies DITAVAL filtering"]
    end

    subgraph Plugin["org.dita.dita2graph — DITA-OT Plugin"]
        AntXml["plugin.xml / build.xml / cfg<br/>registers transtype dita2graph,<br/>declares args.dita2graph.* params"]
        ExtractTask["ExtractTask (Java)<br/>dita2graph:extract Ant task<br/>lib/dita2graph-core.jar"]
        AntXml -->|dispatches| ExtractTask
    end

    subgraph Core["dita2graph-core (Rust binary/crate)"]
        CoreCli["CLI: build · validate · query<br/>relations.rs · okf.rs · rag.rs · secrets.rs"]
    end

    subgraph Store["Generated Bundle (filesystem)"]
        OkfDir[("okf/<br/>OKF concepts + graph.json")]
        RagDir[("rag/<br/>chunks.jsonl + metadata.json")]
        McpToml[("mcp/mcp-server.toml<br/>(optional, --args.dita2graph.mcp)")]
    end

    subgraph Mcp["dita2graph-mcp (Rust binary/crate)"]
        McpServer["MCP server<br/>JSON-RPC over stdio<br/>BundleReader + tools.rs"]
    end

    subgraph Agents["AI Tools & Agents"]
        Claude["Claude Code / Claude Desktop"]
        IDE["VS Code / JetBrains / custom agents"]
    end

    subgraph Dev["Dev / CI harnesses"]
        Gradle["gradle-build/<br/>dita-ot-gradle demo harness"]
        CI["GitHub Actions<br/>rust.yml · java.yml · integration.yml"]
    end

    Repo --> Preprocess
    Preprocess -->|resolved map + job data| ExtractTask
    ExtractTask -->|shells out, normalized model JSON<br/>via DITA2GRAPH_CORE_BIN| CoreCli
    CoreCli -->|writes| OkfDir
    CoreCli -->|writes| RagDir
    CoreCli -->|writes, optional| McpToml
    McpToml -.->|graph.okf path| McpServer
    OkfDir -->|BundleReader::open reads graph.json| McpServer
    RagDir -->|rag_chunks| McpServer
    McpServer <-->|JSON-RPC: initialize, tools/list, tools/call| Claude
    McpServer <-->|JSON-RPC over stdio| IDE

    Gradle -.->|installs + invokes, demo/dev only| Plugin
    CI -.->|builds + tests| Plugin
    CI -.->|builds + tests| Core
    CI -.->|builds + tests| Mcp
```

## Class diagrams — Rust architecture

### `core/dita2graph-core` — normalized model

`model.rs` is plain serde structs and enums — there is **no**
`Graph`/`GraphBuilder` type in the core crate. `okf.rs`'s
`GraphJson`/`GraphNode`/`GraphEdge` types (used to serialize
`graph.json`) are private and write-only, not exported from the crate
and not a queryable in-memory graph.

```mermaid
classDiagram
    class NormalizedNode {
        <<enumeration>>
        Topic
        Map
        +id() &str
        +title() &str
        +links() &[Link]
        +source_file() &str
        +okf_type() &'static str
        +bundle_dir() &'static str
    }

    class NormalizedTopic {
        +id: String
        +topic_type: TopicType
        +title: String
        +shortdesc: Option~String~
        +body: Option~String~
        +audience: Vec~String~
        +product: Vec~String~
        +keys: Vec~String~
        +uicontrols: Vec~String~
        +cmd_uicontrols: Vec~String~
        +source_file: String
        +links: Vec~Link~
    }

    class NormalizedMap {
        +id: String
        +title: String
        +source_file: String
        +links: Vec~Link~
    }

    class TopicType {
        <<enumeration>>
        Concept
        Task
        Reference
        Glossentry
        Topic
        +okf_type() &'static str
    }

    class Link {
        +relation: Relation
        +target: String
    }

    class Relation {
        <<enumeration>>
        Contains
        References
        RelatedTo
        AppliesTo
        Requires
        GeneratedFrom
        +as_str() &'static str
        +section_heading() &'static str
        +needs_frontmatter_extension() bool
    }

    class BundleSummary {
        +topics_written: usize
        +maps_written: usize
        +edges_written: usize
    }

    class RagSummary {
        +chunks_written: usize
    }

    class SecretFinding {
        +file: String
        +pattern: &'static str
    }

    class Severity {
        <<enumeration>>
        Fatal
        Error
        Warning
        Info
    }

    class MessageId {
        +code: &'static str
        +severity: Severity
    }

    NormalizedNode --> NormalizedTopic : Topic variant
    NormalizedNode --> NormalizedMap : Map variant
    NormalizedTopic "1" *-- "*" Link : links
    NormalizedTopic --> TopicType : topic_type
    Link --> Relation : relation
    MessageId --> Severity : severity

    note for NormalizedNode "model.rs — plain serde structs.<br/>No Graph/GraphBuilder type in core;<br/>okf.rs's GraphJson/GraphNode/GraphEdge<br/>are private, write-only serialization shapes."
```

`BundleSummary`, `RagSummary`, and `SecretFinding` are return values of
`write_bundle`, `write_rag_index`, and `scan_bundle` respectively — they
aren't referenced by the model types above, which is why they're
unconnected in the diagram. `MessageId`/`Severity` (`diagnostics.rs`)
back the `DITA2GRAPHnnnX` message catalog used for build diagnostics.

### `mcp/dita2graph-mcp` — bundle reader and tool dispatch

`BundleReader` is **not** shared in-process with `dita2graph-core`'s
writer — it's rebuilt on demand by re-parsing `okf/graph.json` off
disk each time the MCP server opens a bundle.

```mermaid
classDiagram
    class BundleReader {
        -root: PathBuf
        -nodes: HashMap~String, GraphNode~
        -edges: Vec~GraphEdge~
        +open(root: Path) Result~Self~
        +all_nodes() Iterator~GraphNode~
        +edges_from(id, relation) Vec~GraphEdge~
        +edges_to(id, relation) Vec~GraphEdge~
        +concept_path(id) Option~PathBuf~
        +read_concept(id) Result~Value, String~
        +rag_chunks() Result~Vec~RagChunk~~
        +title(id) Result~String~
    }

    class GraphNode {
        +id: String
        +type_: String
    }

    class GraphEdge {
        +from: String
        +to: String
        +relation: String
    }

    class RagChunk {
        +id: String
        +title: String
        +text: Option~String~
        +topic_type: String
    }

    class ToolsDispatch {
        <<module: tools.rs>>
        +list() Vec~Value~
        +call(name, arguments, bundle_root) Result~String~
        search_topics(bundle, args)
        find_related_topics(bundle, args)
        explain_task(bundle, args)
        trace_dependencies(bundle, args)
        search_content(bundle, args)
        analyze_impact(bundle, args)
        generate_summary(bundle, args)
        validate_bundle(bundle_root)
    }

    BundleReader "1" *-- "*" GraphNode : nodes
    BundleReader "1" *-- "*" GraphEdge : edges
    ToolsDispatch ..> BundleReader : queries
    ToolsDispatch ..> RagChunk : reads via rag_chunks
    BundleReader ..> RagChunk : parses rag/chunks.jsonl

    note for BundleReader "Rebuilt per JSON-RPC call by re-parsing<br/>okf/graph.json on disk — not shared<br/>in-process with dita2graph-core's writer."
```

## Activity diagrams — internal processing workflows

### Parsing / extraction (`ExtractTask`, Java)

What happens between DITA-OT resolving a map and the normalized model
being handed off to the Rust core — including `args.dita2graph.depth`
enforcement, `related-links` exclusion, `xtrf`-derived `generated-from`
provenance, and `<navref>` detection (`docs/dev/phase-0-findings.md`
findings 11, 14, 15, 16).

```mermaid
flowchart TD
    Start([Start: dita --format dita2graph]) --> Resolve["DITA-OT preprocess2 / build-init<br/>resolves keys, keyrefs, conref/conkeyref,<br/>topicref hierarchy, applies DITAVAL filtering"]
    Resolve --> Walk["ExtractTask walks resolved job data:<br/>topicref / topichead / topicgroup,<br/>at any nesting depth"]
    Walk --> DepthCheck{"Containment level<br/>within args.dita2graph.depth?"}
    DepthCheck -- "no: node still extracted,<br/>only its contains edge is omitted" --> Metadata
    DepthCheck -- yes --> ContainsEdge["Emit contains edge<br/>from parent map/topic"]
    ContainsEdge --> Metadata["Extract topic metadata:<br/>title, shortdesc, body, audience, product,<br/>keys, uicontrols, cmdUicontrols"]
    Metadata --> ExcludeNav["Exclude DITA-OT's auto-generated<br/>related-links navigation"]
    ExcludeNav --> Provenance["Derive generated-from edges from xtrf<br/>(conref/conkeyref reuse vs. keyref substitution)"]
    Provenance --> NavrefCheck{"&lt;navref&gt; present?"}
    NavrefCheck -- yes --> WarnNavref["Log DITA2GRAPH warning<br/>(detected, not resolved — finding 16)"]
    NavrefCheck -- no --> Emit
    WarnNavref --> Emit["Serialize normalized model<br/>Vec&lt;NormalizedNode&gt; as JSON"]
    Emit --> Handoff(["Shell out to dita2graph-core<br/>(DITA2GRAPH_CORE_BIN)"])
```

### Graph generation (`dita2graph-core build`, Rust)

The `run_build` flow in `core/dita2graph-core/src/main.rs`: relation
inference, bundle writing, optional MCP config, and the two validation
gates (`okf_validator` + secret-leak scan) that make a failed build
fail loudly instead of shipping a bad bundle.

```mermaid
flowchart TD
    Start([Start: dita2graph-core build]) --> Parse["Read normalized model JSON<br/>Vec&lt;NormalizedNode&gt;"]
    Parse --> AppliesTo["infer_applies_to<br/>(relations.rs)"]
    AppliesTo --> RelatedTo["infer_related_to<br/>(relations.rs)<br/>ambiguous matches dropped + logged, not guessed"]
    RelatedTo --> WriteOkf["write_bundle<br/>(okf.rs) → okf/ concepts + graph.json"]
    WriteOkf --> WriteRag["write_rag_index<br/>(rag.rs) → rag/chunks.jsonl + rag/metadata.json"]
    WriteRag --> McpFlag{"--mcp true?"}
    McpFlag -- yes --> WriteMcp["write_mcp_config<br/>(mcp_config.rs) → mcp/mcp-server.toml"]
    McpFlag -- no --> ValidateOkf
    WriteMcp --> ValidateOkf["validate_and_report(okf/)<br/>okf_validator::validate_bundle + scan_bundle"]
    ValidateOkf --> OkfPass{"Valid +<br/>no secrets found?"}
    OkfPass -- no --> Fail(["Build fails<br/>errors reported, non-zero exit"]):::fail
    OkfPass -- yes --> ScanRag["scan_rag_and_report(rag/)<br/>scan_bundle for secret leakage"]
    ScanRag --> RagPass{"No secrets<br/>in rag/?"}
    RagPass -- no --> Fail
    RagPass -- yes --> Done(["Bundle ready:<br/>okf/ + rag/ (+ mcp/ if requested)"])

    classDef fail fill:#5c1a1a,stroke:#ff6b6b,color:#fff
```

### Incremental update (planned — not yet implemented)

**This is a design diagram, not a working feature.** Source-hash-keyed
incremental rebuild is listed as future work in
`docs/plugin-specification.md` §3.3 and `Roadmap.md`'s Phase 6+ —
today, every `dita2graph-core build` re-extracts and rewrites the whole
bundle. The dashed styling below marks every step as not yet
implemented.

```mermaid
flowchart TD
    Start(["Start: dita2graph-core build<br/>(re-run against an existing bundle)"]) --> HashSrc["Hash each source DITA file<br/>(source-hash keyed, §3.3)"]
    HashSrc --> Compare{"Hash matches the<br/>existing bundle's record?"}
    Compare -- "yes: unchanged" --> Skip["Skip re-extraction for this topic;<br/>reuse existing okf/ concept + rag/ chunk"]
    Compare -- "no: changed or new" --> FullExtract["Run full extraction for this topic<br/>(§3.2 normalized model → relations → OKF/RAG write)"]
    FullExtract --> Diff["Diff against existing bundle:<br/>updated / added / removed nodes and edges"]
    Skip --> Merge
    Diff --> Merge["Merge into bundle:<br/>update graph.json, okf/ concepts, rag/chunks.jsonl"]
    Merge --> Stale["Refresh generated.at frontmatter<br/>for changed concepts only"]
    Stale --> Done(["Incrementally updated bundle"])

    classDef planned stroke-dasharray: 5 5,stroke:#888,color:#888,fill:transparent
    class Start,HashSrc,Compare,Skip,FullExtract,Diff,Merge,Stale,Done planned
```

## Sequence diagram — AI agent ↔ MCP server ↔ knowledge graph

A concrete walk through `docs/plugin-specification.md` §5.3's example
interaction, down to the actual JSON-RPC methods `dita2graph-mcp`
implements (`initialize`, `tools/list`, `tools/call`) and the
`BundleReader` calls each tool makes (`mcp/dita2graph-mcp/src/tools.rs`).

```mermaid
sequenceDiagram
    actor User
    participant Agent as AI Agent / IDE<br/>(Claude Code)
    participant MCP as dita2graph-mcp<br/>(JSON-RPC over stdio)
    participant Reader as BundleReader
    participant KG as Knowledge Graph<br/>(okf/graph.json, rag/chunks.jsonl)

    Agent->>MCP: initialize
    MCP-->>Agent: protocolVersion "2024-11-05" + capabilities

    Agent->>MCP: tools/list
    MCP-->>Agent: search_topics, search_content, find_related_topics,<br/>explain_task, trace_dependencies, analyze_impact,<br/>generate_summary, validate_bundle

    User->>Agent: "How do I configure authentication?"
    Agent->>MCP: tools/call search_topics(query="authentication configuration")
    MCP->>Reader: open(bundle_root)
    Reader->>KG: read graph.json
    KG-->>Reader: nodes + edges
    Reader-->>MCP: BundleReader (in-memory index)
    MCP->>Reader: match query against titles/ids
    Reader-->>MCP: Authentication Configuration (Concept)
    MCP-->>Agent: topic match

    Agent->>MCP: tools/call find_related_topics(topicId, relation?)
    MCP->>Reader: edges_from(topicId, relation)
    Reader-->>MCP: requires: Security Module, User Database<br/>related-to: Configuring SSO, Rotating API Keys
    MCP-->>Agent: related concepts

    Agent-->>User: Requires Security Module + User Database,<br/>see also Configuring SSO, Rotating API Keys
```
