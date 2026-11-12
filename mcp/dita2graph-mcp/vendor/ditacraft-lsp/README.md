# Vendored: DitaCraft standalone LSP server

**Source:** [`jyjeanne/ditacraft`](https://github.com/jyjeanne/ditacraft)
**Version vendored:** v0.11.0 (commit `39b0fdc4dbdac1ace32035784541c154f722e830`,
2026-11-11 release; `dist/lsp-server.js` downloaded from that release's
`lsp-server-0.11.0.zip` GitHub Release asset rather than rebuilt from
source, to vendor exactly what upstream shipped — zip SHA-256
`63f7b0a75c434e37fa3ee912a0397d1e5cb6a17bb7437944ac7ef3453bf9d90c`,
the digest GitHub publishes for the asset; `lsp-server.js` SHA-256
`df17872ffc8b23cd8501d8ebc42b0f83cefafd8a9c5c7a2e2b7894ff9d7ec090`).
v0.11.0 is a DITA 1.3 conformance release, checked against DITA-OT 4.3.5.
What changes for `validate_live`:
- Key references resolve across key scopes as DITA 1.3 requires: a
  parent scope's definition wins over a child scope's, a key defined
  only in a child scope is reachable from outside it by its
  scope-qualified name only, and a peer map's keys are named by the
  `@keyscope` of the map reference.
- Reference severities follow DITA-OT: a missing DITA topic, map,
  content reference or image (`DITA-XREF-001`), a content reference
  whose target cannot be found (`DITA-XREF-002`/`003`, `DITA-KEY-003`)
  and a `@conkeyref` to an undefined key (`DITA-KEY-001`) are now
  errors, no longer warnings. `DITA-SCH-003` (`<indextermref>`) drops
  to a warning and `DITA-SCH-014` (`@navtitle`) to information.
- Content reference compatibility is decided by `@class`
  (`DITA-XREF-004`), and `@conrefend` ranges are checked
  (`DITA-XREF-006`, `DITA-XREF-007`).
- Fewer false reports: URI-encoded `@href`s, ids in non-ASCII scripts,
  specialized topic types as conref targets, subject scheme
  `<subjectdef>` keys, documents tied to RELAX NG by `<?xml-model?>`.

v0.10.4 (2026-10-26), between the two, only fixed subject scheme maps
without a DOCTYPE in the editor and updated dependencies.
**DTDs:** `dtds/` matches the same tag; its 327 files are unchanged
since v0.10.3: the `dtds` tree has the same Git SHA
(`8cea000155c45ae8432b58a4255802649105eb83`) in v0.10.3, v0.10.4 and
v0.11.0. v0.10.2 had only changed `dtds/base/catalog.xml` since v0.9.1
(adds the missing DITA 1.3 `topicDefn.ent` mapping, which otherwise
suffix-matched the DITA 1.2 copy, whose common elements lack `<div>`)
**License:** MIT (`jyjeanne/ditacraft`'s `LICENSE`) — compatible with
DITA2Graph's own dual MIT OR Apache-2.0. See root `NOTICE`.
**Known issues in this specific vendored build:** see `KNOWN-ISSUES.md`
in this directory (none currently open — v0.9.1 fixed the one
previously tracked there, `jyjeanne/ditacraft#125`).

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

Two equally valid ways to get a fresh `dist/lsp-server.js`, matching
exactly what `jyjeanne/ditacraft`'s own release workflow produces
either way:

**Preferred — download the release asset directly** (what the v0.9.1,
v0.10.2, v0.10.3 and v0.11.0 updates did), so what's vendored here is byte-identical to
what upstream actually shipped, not a local rebuild that merely
*should* match:

```bash
curl -sSLO https://github.com/jyjeanne/ditacraft/releases/download/vX.Y.Z/lsp-server-X.Y.Z.zip
unzip -o lsp-server-X.Y.Z.zip   # -> lsp-server.js
```

**Or rebuild from source**, e.g. if you need an unreleased commit:

```bash
npm ci && (cd server && npm ci)
node esbuild-standalone.js --minify   # matches release.yml's own build step
```

Either way, replace this directory's `dist/lsp-server.js` and `dtds/`
(only needed if the DTD set itself changed upstream) with the fresh
ones, and update the version/commit/date at the top of this file plus
the `NOTICE` entry. Then re-run
`cargo test -p dita2graph-mcp` (the full suite, not just `live::`) —
`validate_file_no_longer_hangs_on_a_real_codeblock_from_dita_ot_docs`
in particular is a standing regression guard against the one bug this
vendored copy has already had. `dist/mcp-server.js` (DitaCraft's own
separate MCP server) is a sibling build output but is *not* vendored
here — it exposes a different tool set
(`docs/mcp-server-implementation.md` in that repo) meant to be
registered as its own independent MCP server (see
`lsp-dita-integration-plan.md` at this repo's root, Option B), not
embedded inside `dita2graph-mcp`.
