# DITA LSP (DitaCraft) Integration Study & Plan

**Subject repo studied:** [`jyjeanne/ditacraft`](https://github.com/jyjeanne/ditacraft) (v0.9.0)
**Target repo:** `jyjeanne/DITA2Graph` (this repo, v0.1.0)
**Author of both projects:** Jeremy Jeanne — no cross-org licensing or governance
friction; both are MIT-family licensed (DitaCraft: MIT; DITA2Graph: dual
MIT OR Apache-2.0), so any code reuse across them is unencumbered.
**Status:** Study + proposed plan, no implementation yet.

---

## 1. Executive summary

DitaCraft and DITA2Graph solve two different halves of "give an AI agent
(and a human writer) real understanding of a DITA content set," and they
currently solve them **completely independently**, with no shared code,
no shared data format, and no awareness of each other:

| | DitaCraft (`ditacraft` LSP + MCP) | DITA2Graph (this repo) |
|---|---|---|
| **When it runs** | Live, in-editor, as-you-type | Build-time, via DITA-OT `--format dita2graph` |
| **Scope** | One open workspace, current document state | A resolved, DITAVAL-filtered publication of the whole map |
| **Core artifact** | In-memory key space / diagnostics (ephemeral) | Persisted OKF bundle (`okf/graph.json`, `rag/chunks.jsonl`) on disk |
| **Language/runtime** | TypeScript, Node.js, VS Code LSP (IPC) | Rust core + Java DITA-OT plugin |
| **Relations it knows** | href/conref/keyref/conkeyref resolution, key space (BFS + keyscope PushDown) | `contains`/`requires`/`references`/`applies-to`/`related-to`/`generated-from` typed graph edges |
| **AI surface** | Its own MCP server (6 tools: validate, snapshot, key-space, map-structure, resolve-reference, explain-key) + `@ditacraft` Copilot chat participant | Its own MCP server (8 tools: search_topics, search_content, find_related_topics, explain_task, trace_dependencies, analyze_impact, generate_summary, validate_bundle) |
| **Best at** | "Is this document valid right now, and where does this keyref/conref point?" | "What depends on this topic transitively, across the whole doc set, with content excerpts?" (`analyze_impact` — DitaCraft has no equivalent) |
| **Weak at** | No transitive dependency graph, no cross-document relation inference (`applies-to`, `related-to`, `generated-from`), no RAG-ranked content search | No live, sub-second, as-you-type feedback; no editor UI; nothing runs until a build is triggered |

**The recommendation is not to merge the two codebases.** They're built
in different languages for different execution contexts (editor-process
TypeScript vs. build-time Rust/Java), and forcing a merge would just
create a slower, harder-to-maintain version of each. Instead, the plan
is **protocol-level federation**: both already speak MCP over stdio,
which is exactly the integration seam both projects' own docs point at
(DITA2Graph's `README.md` workflow diagram ends in "AI agent / IDE
(Claude Code, Claude Desktop, custom agents)"; DitaCraft's `README.md`
already lists "opencode, Claude Desktop, Cursor, Continue" as compatible
MCP clients). Wiring DITA2Graph's MCP server into DitaCraft's VS Code
extension — as a second, complementary MCP server the extension helps
register and keep fresh — gets a technical writer and their AI agent
both halves (live authoring correctness + whole-doc-set graph
intelligence) inside one editor session, for a bounded, low-risk amount
of new code.

---

## 2. What each project actually is (evidence from the code)

### 2.1 DITA2Graph (this repo)

- A **DITA-OT plugin** (`plugin/org.dita.dita2graph`) registers a
  `dita2graph` transtype. `dita --format dita2graph` runs DITA-OT's
  normal preprocessing (key/conref/keyref resolution, DITAVAL filtering,
  map flattening) and then dispatches to a Java `ExtractTask`.
- `ExtractTask` walks the *resolved* map tree (any nesting depth) and
  serializes a `NormalizedNode` model (topics + maps, with typed
  `Link`s) as JSON, then shells out to the Rust `dita2graph-core`
  binary.
- `dita2graph-core` infers additional relations not present in the raw
  DITA source (`applies-to` from `uicontrols`, `related-to`, and
  `generated-from` from DITA-OT's own `xtrf` source-trace attributes —
  i.e., it can tell a real `conref`/`conkeyref` reuse from a `keyref`
  substitution), writes an **OKF v0.2 bundle** (`okf/` — markdown
  concepts + `graph.json`) and a **RAG content index**
  (`rag/chunks.jsonl` + `rag/metadata.json`), and validates both against
  `okf_validator` plus a build-breaking secret-leak scanner.
- `dita2graph-mcp` reads that bundle back off disk (`BundleReader`,
  rebuilt per JSON-RPC call — no persistent in-process index) and serves
  8 tools over stdio JSON-RPC to any MCP client.
- **This is fundamentally a build artifact + query server**, not an
  editor tool. Nothing in this repo runs while someone is typing.

### 2.2 DitaCraft (`jyjeanne/ditacraft`)

- A **VS Code extension** (`src/`) that activates on `.dita`/`.ditamap`
  files, wires up a **standard LSP server** (`server/`, TypeScript,
  `vscode-languageserver` 9.x, IPC transport) providing pull diagnostics,
  completion, hover, go-to-definition, find-references, rename,
  formatting, folding, linked editing, and document links — 703 tests.
- The LSP's `ValidationPipeline` runs a **13-phase pipeline**
  (well-formedness → DTD/RNG → cross-reference → 43 Schematron-equivalent
  rules → circular-reference DFS → subject-scheme profiling →
  workspace-level duplicate/orphan checks → custom regex rules →
  comment-based suppression) on a 300ms (topic) / 1000ms (map) debounce,
  entirely in-memory, entirely local to the open workspace.
- `KeySpaceService` does its own **DITA 1.3-spec-compliant BFS key space
  resolution** — `mapref`/`keyscope` PushDown inheritance, provenance
  tracking, a 50k scope-explosion cap — independently of, and more
  thoroughly than, DITA2Graph's Java extraction (which only records
  `keys: Vec<String>` per topic, no keyscope/PushDown modeling at all).
- A **separate** standalone MCP server (`mcp/`, esbuild-bundled, zero VS
  Code dependency) re-exposes that same live workspace state
  (`ditaValidate`, `ditaContextSnapshot`, `ditaKeySpace`,
  `ditaMapStructure`, `ditaResolveReference`, `ditaExplainKey`) to
  external agents — this is DitaCraft's own prior art for "give an AI
  agent MCP access to DITA intelligence," built completely independently
  of DITA2Graph, using none of its OKF/graph vocabulary.
- Also has an AI layer on top (`src/llm/`): a Copilot-chat participant
  (`@ditacraft` with `/restructure`, `/validate`, `/explain`,
  `/suggest-reuse`), AI quick fixes, AI completion, and a
  Copilot→Anthropic→OpenAI→Ollama provider cascade with a circuit
  breaker — none of which currently has access to any *cross-document*
  relation data, because the LSP only ever looks at what key space
  resolution and regex/DFS scanning over open files can find.

### 2.3 The gap between them, concretely

`/suggest-reuse` (DitaCraft's chat command for finding conref/keyref
reuse opportunities) can only work off files currently reachable via
live key-space traversal and ad hoc scanning. DITA2Graph's
`generated-from` edges are *provably correct* reuse provenance, derived
from DITA-OT's own `xtrf` trace attributes at build time — something the
LSP cannot reconstruct without literally reimplementing DITA-OT's
conref-resolution internals. Conversely, `analyze_impact` — "what
transitively depends on this topic, with content excerpts" — has no
equivalent anywhere in DitaCraft; nothing in the LSP builds or persists a
reverse dependency graph across the whole doc set. Each side has data
the other structurally cannot produce.

---

## 3. Benefits of integrating

1. **`analyze_impact`/`trace_dependencies` inside the editor.** Today a
   DitaCraft user has no way to ask "what breaks if I rename/retire this
   topic?" without leaving VS Code. Wiring DITA2Graph's MCP tools into
   the `@ditacraft` chat participant (or a new command) answers that
   from real graph edges, in the same place they're already editing.
2. **Provenance-correct reuse suggestions.** `/suggest-reuse` gets
   strictly better if it's grounded in `generated-from` edges instead of
   re-deriving conref reuse from scratch.
3. **A pre-publish gate DitaCraft doesn't have.** DITA2Graph's build
   already runs `okf_validator` + a secret-leak scanner as a hard build
   gate. Surfacing "Build Knowledge Graph" as a DitaCraft command with a
   report panel (mirroring the existing `ValidationReportPanel`) gives
   writers a pre-publish structural + secret-leak check for free,
   reusing UI DitaCraft already has.
4. **One-agent, two-server MCP setup that already fits the ecosystem.**
   Both projects list the same class of MCP clients (Claude Code, Claude
   Desktop, Cursor/Continue/opencode). Registering both servers together
   is additive, not a redesign — an agent gains graph/RAG tools without
   losing any live-authoring tools.
5. **No code fork, no format war.** DITA2Graph owns OKF/graph
   vocabulary; DitaCraft owns LSP/live-workspace vocabulary. Neither
   needs to adopt the other's internal types — MCP is already the
   contract boundary both chose independently.
6. **Feeds DITA2Graph real dogfood usage.** DITA2Graph's own
   `Roadmap.md`/`docs/plugin-specification.md` §13.2 lists federation,
   HTTP transport, and "AI-tool interaction" as forward-looking, but has
   no editor-embedded consumer today — every current usage example is a
   raw CLI/stdio JSON-RPC snippet. An editor integration is the first
   real end-user surface beyond "hand-roll `claude mcp add`."

---

## 4. Integration options considered

### Option A — Documentation-only federation (near-zero effort)

Just document, in both repos, how to register both MCP servers with the
same agent (e.g., two entries in Claude Code's `mcp.json` / VS Code's
`.vscode/mcp.json`). No code changes. Ships value immediately but
requires the user to manually keep the DITA2Graph bundle rebuilt, and
gives no in-editor UI.

### Option B — DitaCraft-side MCP client + commands (recommended, see §5)

DitaCraft (the consumer with the existing UI/command/webview
infrastructure) adds:
- A command to build/refresh a DITA2Graph bundle via the same
  `ditaOtWrapper.ts` child-process pattern it already uses for
  publishing (`DitaOtWrapper.execute`-style call to
  `dita --format dita2graph`).
- A command/setting to register `dita2graph-mcp <bundlePath>` as an MCP
  server, either by writing a `.vscode/mcp.json` entry (VS Code's native
  MCP client, used by Copilot Chat) or by spawning it directly as a
  stdio child process from a new provider class parallel to
  `aiServiceOrchestrator.ts`.
- Optional: extend `@ditacraft`'s `/suggest-reuse` and a new
  `/impact` chat command to call the DITA2Graph tools when a bundle is
  present, falling back to today's live-scan behavior when it isn't.

This is additive to DitaCraft, requires **zero** changes to DITA2Graph
(it already speaks correct MCP), and reuses infrastructure DitaCraft
already has (child-process wrapper, webview report panels, chat
participant, settings/config UI).

### Option C — DITA2Graph-side "editor mode" (rejected for now)

Have DITA2Graph grow its own LSP-like live layer to match DitaCraft's.
Rejected: this duplicates ~703 tests' worth of DitaCraft functionality
in Rust/Java for no benefit — DITA2Graph's own `Roadmap.md` doesn't even
have incremental/live rebuilding yet (`docs/architecture.md`'s
"Incremental update" activity diagram is explicitly marked **planned,
not implemented**), so a live editor layer on top of it is premature.

### Option D — Deep type/algorithm sharing (rejected for now)

Port DitaCraft's `KeySpaceService` (full keyscope PushDown, provenance,
scope-explosion cap) into DITA2Graph's Java `ExtractTask`, since
DITA2Graph's own key modeling is currently just a flat `keys:
Vec<String>` per topic with no keyscope awareness. Cross-language port
(TypeScript → Java) is real value but is a separable, DITA2Graph-only
improvement — it doesn't require or benefit from DitaCraft integration
first, so it's tracked as a follow-up (§7) rather than blocking this
plan.

---

## 5. Recommended plan (Option B), phased

### Phase 1 — Bundle lifecycle command in DitaCraft

- Add `DitaCraft: Build Knowledge Graph` command, implemented the same
  way `publishHTML5Command`/`watchModeCommand` already invoke DITA-OT
  (via `DitaOtWrapper`), but with `transtype: 'dita2graph'` and the
  `args.dita2graph.*` params from `plugin.xml` surfaced as DitaCraft
  settings (`ditacraft.dita2graph.depth`, `.mcp`, etc.).
- Reuse the existing DITA-OT error-parsing/Problems-panel pipeline for
  build failures (including `okf_validator` / secret-leak failures,
  which already produce structured, non-zero-exit failures per
  DITA2Graph's own `docs/architecture.md` activity diagram).
- Output: a bundle under the workspace (e.g.
  `.dita2graph/okf/`, `.dita2graph/rag/`, `.dita2graph/mcp/mcp-server.toml`
  — the last written automatically when `--args.dita2graph.mcp true` is
  passed, per DITA2Graph's §5.4).

### Phase 2 — MCP registration

- Add `DitaCraft: Register DITA2Graph MCP Server` command that writes/
  updates a `.vscode/mcp.json` entry pointing at the built
  `dita2graph-mcp` binary + bundle root (or `--config mcp-server.toml`),
  so VS Code's native MCP client (and therefore Copilot Chat) picks it
  up with no custom stdio-client code needed in DitaCraft.
- Document the manual equivalent (`claude mcp add dita2graph -- ...`)
  for Claude Code users, matching DITA2Graph's own README quickstart.
- Gate this behind a bundle actually existing (Phase 1 having run) with
  a clear prompt if it hasn't.

### Phase 3 — Freshness

- Extend `watchModeCommand.ts`'s existing watch pattern (already watches
  DITA source files and re-runs a full publish on change) to optionally
  also re-run the Phase 1 build on the same trigger, keeping the graph
  reasonably fresh during an editing session. Explicitly **not**
  incremental (DITA2Graph doesn't support that yet — full rebuild each
  time, exactly like `watchModeCommand.ts`'s own documented "not
  incremental" caveat for publishing).
- Debounce/status-bar UX mirrors the existing watch-mode status bar item
  (`Watching` / `Building...` / `Built` / `Build failed`).

### Phase 4 — Chat participant enrichment (stretch)

- `/suggest-reuse`: when a bundle is present and fresh, query
  `find_related_topics`/`generated-from` edges via the registered MCP
  server instead of (or in addition to) today's live scan; fall back
  gracefully when no bundle exists.
- New `/impact` chat command wrapping `analyze_impact` directly —
  "what depends on this topic" as a first-class chat command, since
  DITA2Graph's own README calls this out as the standout use case that
  plain-text RAG structurally cannot do.
- All of this calls the *already-registered* MCP server from Phase 2 —
  no new protocol code, just prompt/tool wiring in
  `src/chat/ditacraftParticipant.ts`.

### Non-goals for this plan

- No changes to DITA2Graph's Rust/Java code are required for Phases
  1–4 — it already does everything asked of it correctly over MCP.
- No attempt to unify the two MCP tool sets into one server, or to make
  DitaCraft's own MCP server and DITA2Graph's MCP server merge/proxy
  each other. They stay two independent servers an agent (or VS Code)
  can register side by side.

---

## 6. Risks and open questions

| Risk | Notes |
|---|---|
| **Staleness** | The graph is only as current as the last build; a writer mid-edit sees live LSP diagnostics but a possibly-stale graph. Phase 3's watch mode mitigates but doesn't eliminate this — needs a visible "bundle built N minutes ago" indicator so agents/writers don't trust stale `analyze_impact` results silently. |
| **DITA-OT dependency** | DITA2Graph requires a real DITA-OT 4.4 install (its own README: "first run downloads DITA-OT... ~80MB installed"). DitaCraft already depends on DITA-OT for publishing, so this isn't a *new* dependency for DitaCraft users, but it does mean Phase 1 doesn't work for a DitaCraft user who has never configured DITA-OT. |
| **Toolchain footprint** | DITA2Graph needs Rust/Cargo-built binaries (`dita2graph-core`, `dita2graph-mcp`) present on the machine, or DitaCraft would need to ship/download prebuilt binaries (DITA2Graph's README mentions its release workflow publishes Rust binaries — worth checking those are cross-platform release artifacts DitaCraft's command can fetch, rather than requiring a local Rust toolchain). |
| **Two MCP servers, overlapping-sounding tools** | Both projects have a `validate`-shaped tool (DitaCraft's `ditaValidate` vs. DITA2Graph's `validate_bundle`) with different meanings (live single-file/fragment validation vs. re-running the build-time validator on the last-built bundle). An agent given both needs tool descriptions precise enough to disambiguate — worth double-checking tool descriptions on both sides read unambiguously side by side. |
| **HTTP transport gap** | DITA2Graph's HTTP transport (needed for anything beyond a single local editor session, e.g. a shared team MCP endpoint) is explicitly **not yet implemented** (§6.3) — this plan is scoped to the stdio/local-editor case only. |
| **License** | Both permissive (MIT / MIT-OR-Apache-2.0) — no blocker. Confirmed by reading `LICENSE` in both repos. |

---

## 7. Follow-up work (out of scope for this plan, tracked for later)

- **Key-space algorithm upgrade for DITA2Graph** (Option D, §4): port
  DitaCraft's keyscope PushDown/provenance-aware key space resolution
  into DITA2Graph's Java `ExtractTask`, replacing the current flat
  `keys: Vec<String>`. Independently valuable, not integration-specific.
- **DITA2Graph incremental builds**: would make Phase 3's watch-mode
  rebuild cheap enough to run on every save instead of on a debounce;
  currently blocked on DITA2Graph's own not-yet-implemented
  source-hash-keyed incremental update (`docs/architecture.md`).
- **A DITA-OT HTML5/PDF plugin variant with graph-backed "related
  topics" panels** — already listed in DITA2Graph's own §13.2 as a
  future direction; would pair naturally with DitaCraft's existing Live
  Preview panel if DitaCraft's preview WebView called
  `find_related_topics` for the currently-previewed topic.
