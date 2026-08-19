# Vendored: DitaCraft standalone LSP server

**Source:** [`jyjeanne/ditacraft`](https://github.com/jyjeanne/ditacraft)
**Version vendored:** v0.9.0 (commit `abc5070bd4d07610e93094d3f2e35d527b5bd61a`,
2026-08-15 release, per that repo's `CHANGELOG.md`)
**License:** MIT (`jyjeanne/ditacraft`'s `LICENSE`) — compatible with
DITA2Graph's own dual MIT OR Apache-2.0. See root `NOTICE`.

## What this is

`dist/lsp-server.js` is DitaCraft's **standalone LSP server bundle** —
the same build its own release workflow
(`jyjeanne/ditacraft/.github/workflows/release.yml`) attaches to every
GitHub Release as `lsp-server-<version>.zip`. It's a single, minified,
self-contained Node.js file (esbuild, `format: cjs`, `platform: node`)
bundling the full DITA Language Server: 13-phase validation pipeline
(XML well-formedness, DTD via TypesXML, optional RelaxNG, 43
Schematron-equivalent rules, cross-reference/key-space resolution,
circular-reference detection, subject-scheme profiling), completion,
hover, and the rest of `docs/DITA_LSP_ARCHITECTURE.md`'s LSP capability
table in the source repo. No VS Code dependency — it speaks plain LSP
JSON-RPC over `--stdio`, per its own `server/src/standalone.ts` doc
comment:

```
Usage:  node dist/lsp-server.js --stdio
```

`dtds/` is the DITA 1.2/1.3/2.0 DTD set the DTD-validation phase reads
via an OASIS XML Catalog (`dtds/catalog.xml`) — required alongside the
bundle; `DITACRAFT_EXTENSION_ROOT` (set by
`mcp/dita2graph-mcp/src/live.rs` when it spawns this process) must point
at this directory (`vendor/ditacraft-lsp/`, the parent of both `dist/`
and `dtds/`), matching `standalone.ts`'s own default resolution
(`path.resolve(__dirname, '..')`).

## Why vendored, not adapted source

`mcp/dita2graph-mcp/src/main.rs`'s stdio transport was **adapted** from
`jyjeanne/okf-rs`'s `okf-mcp` crate — copied, translated, and credited
line-by-line, because both are Rust and the pattern ports directly (see
`docs/plugin-specification.md` §5.5, `NOTICE`).

DitaCraft's LSP server can't be adapted the same way: it's ~40K lines of
TypeScript (`server/src/`, 26 feature/service/util modules, 703 tests)
implementing DITA-specific validation logic DITA2Graph has no Rust
equivalent of and no reason to duplicate. The integration seam here is
the **LSP wire protocol** (`Content-Length`-framed JSON-RPC, LSP 3.17's
pull-diagnostics model), not shared source — `live.rs` is a small,
original Rust LSP *client* that talks to this bundle exactly as
DitaCraft's own VS Code client does, documented in
`docs/DITA_LSP_ARCHITECTURE.md`: "Diagnostics are pull-based (LSP 3.17
— the client requests them)."

## How it's used

`mcp/dita2graph-mcp/src/live.rs`'s `validate_file()` spawns
`node dist/lsp-server.js --stdio` as a child process per call, runs the
`initialize` → `initialized` → `textDocument/didOpen` →
`textDocument/diagnostic` → `shutdown`/`exit` handshake over
`Content-Length`-framed stdio, and returns the resulting `Diagnostic[]`.
This backs the `validate_live` MCP tool (`tools.rs`) — a topic's
*current, on-disk source*, checked live, alongside `validate_bundle`'s
existing re-check of the *last build's* OKF/secret-leak gates.

Requires a `node` binary on `PATH` (or `--node-bin`/`DITA2GRAPH_NODE_BIN`
pointing at one) — Node.js itself is not vendored.

## Updating this vendored copy

From a checkout of `jyjeanne/ditacraft` at the release tag to pick up:

```bash
npm ci && (cd server && npm ci)
node esbuild-standalone.js --minify   # matches release.yml's own build step
```

Then replace this directory's `dist/lsp-server.js` and `dtds/` with the
freshly built ones, and update the version/commit/date at the top of
this file plus the `NOTICE` entry. `dist/mcp-server.js` (DitaCraft's own
separate MCP server) is a sibling build output but is *not* vendored
here — it exposes a different tool set
(`docs/mcp-server-implementation.md` in that repo) meant to be
registered as its own independent MCP server (see
`lsp-dita-integration-plan.md` at this repo's root, Option B), not
embedded inside `dita2graph-mcp`.
