# Roadmap

Phase-by-phase status for DITA2Graph, condensed from
[`docs/plugin-specification.md`](docs/plugin-specification.md) §11
(MVP scope) and §12 (development phases) — that document is the
source of truth with full detail on deliverables and exit criteria per
phase; this page is the scannable summary plus what's actually left.
The evidence behind every "done" claim below — what was verified
against a live DITA-OT 4.4, not just written down — lives in
[`docs/dev/phase-0-findings.md`](docs/dev/phase-0-findings.md), numbered
findings 1–16.

## Phase overview

| Phase | Focus | Status |
|---|---|---|
| 0 | Spike & feasibility | ✅ Done |
| 1 | Plugin skeleton + normalized model | ✅ Done |
| 2 | Core engine: OKF bundle generation | ✅ Done (dedup/incremental rebuild deferred to Phase 6+) |
| 3 | MCP server | ✅ Done (Resources deferred, see below) |
| 4 | Gradle integration + CI hardening | ✅ Done |
| 5 | Security, docs, public `v0.1.0` release | ✅ Done |
| 6+ | Extended capabilities (post-MVP) | 🔄 Ongoing, picked up item by item |

The MVP scope (§11) — plugin, relation extraction, a conformant `okf/`
bundle, an MCP server, a Gradle CI harness, and the public/internal
DITAVAL split — is functionally complete and verified end to end
against a live DITA-OT 4.4 install, not just designed. `v0.1.0` is
tagged from this state.

## Phase detail

### Phase 0 — Spike & feasibility

Confirmed both blocking unknowns before committing to the architecture:
`okf-core`/`okf-validator` are usable as library dependencies (not just
CLI tools), and DITA-OT's own preprocessing resolves `keyref`/`conref`/
DITAVAL filtering before this plugin ever needs to. `okf-dita`/
`okf-generator` turned out *not* reusable for the bundle-write path —
`dita2graph-core` writes its own OKF bundle instead (a documented
fallback decision, not a silent deviation).

### Phase 1 — DITA-OT plugin skeleton + normalized model

`org.dita.dita2graph` installs via `dita --install`, dispatches, and
produces the normalized DITA model end to end against a live DITA-OT
4.4 (findings 5–7 caught and fixed seven real integration bugs along
the way — illegal `--` in XML comments, nonexistent extension points,
Ant target-naming conventions, missing CLI `<param>` declarations, and
more).

Map extraction walks `topicref`/`topichead`/`topicgroup` nesting at any
depth (finding 11), excludes DITA-OT's auto-generated `related-links`
navigation from cross-reference extraction, and supports `mapref`/
`anchorref` submap composition with zero extra code — DITA-OT's own
preprocessing already flattens it into the same map tree (finding 14).
`<navref>` is genuinely unsupported (would mean this plugin
independently parsing/merging navigation maps outside DITA-OT's own
pipeline) but is now *detected* and logged (`DITA2GRAPH060W`) instead
of silently dropped (finding 16). All five `args.dita2graph.*` CLI
parameters (`depth`, `mcp`, `emit-graph-json`, `store`,
`include-drafts`) are functionally wired, not just accepted and logged
(findings 10, 12).

**Found and fixed against a real third-party corpus** (`dita-ot/docs`,
the DITA-OT project's own documentation — 267 `.dita` files, deeply
nested `mapref` composition, real `keyref`/`conref` reuse; a scale and
messiness no hand-built fixture reaches):

- **Duplicate topic ids silently destroyed content.** 33 separate
  topics in that corpus share the literal `id="ID"` (an unfilled
  authoring-template placeholder), plus several genuine `id` reuses
  across near-duplicate topics (e.g. `dita_ot_day_videos_intro` reused
  across multiple years' intro topics). Every topic's graph node id
  came straight from its own `id` attribute with no uniqueness check,
  so colliding ids collapsed onto the same OKF concept file path
  (`okf/topics/{id}.md`) — the second topic written silently
  overwrote the first's content, no error, no warning. Confirmed
  directly: running the un-fixed extractor against `dita-ot/docs`
  produced 33 nodes all named `"ID"` in `graph.json`, but only *one*
  topic's actual text survived in the bundle. Now detected and
  disambiguated (`DITA2GRAPH070W`, naming both colliding files) —
  every topic keeps its own distinct node, none dropped. Re-running
  against the same corpus: 226 nodes, 226 distinct ids, zero
  collisions remaining.
- **A keyref-resolved external topicref produced false-positive
  "unresolved" warnings.** `dita-ot/docs`'s conference-talk maps use
  `<keydef href="https://..." scope="external">` entries (e.g. "watch
  this talk") referenced via `<topicref keyref="...">` — DITA-OT
  resolves the keyref onto the topicref during preprocessing, same as
  any other keyref, so by the time this plugin sees it it's an
  ordinary external `<topicref>`. `<xref>`/`<link>` handling already
  skipped `scope="external"`/`http(s)://` targets; the map-topicref
  `contains`-edge walk had no equivalent check, so every external
  topicref produced a `DITA2GRAPH010W` "unresolved topicref target" —
  there was never a local topic for it to be, so this was noise, not a
  real gap. Fixed with a shared `isExternal` check used by both code
  paths — dozens of false-positive warnings gone, zero change to real
  unresolved-reference detection.

Both are exactly the kind of gap only real content surfaces — neither
pattern (a template's unfilled placeholder id reused across dozens of
copy-pasted topics; an external-link keydef) exists in this project's
own hand-built fixtures, and both are common enough in real,
multi-contributor DITA corpora that a tool claiming real-dataset
readiness needs to handle them without losing data or crying wolf.

### Phase 2 — Core engine: OKF bundle generation

`dita2graph-core` normalizes the model, writes a conformant `okf/`
bundle plus derived `graph.json`, and infers every relation in the
taxonomy. `contains`/`requires`/`references` come straight from
authored markup; `generated-from` is derived deterministically from
DITA-OT's own `xtrf` source-trace attributes, no inference needed
(finding 15); `related-to` (shared `product` values, finding 13) and
`applies-to` (matching `<uicontrol>` text between a task and a
reference topic, with an ambiguous match dropped and logged rather than
guessed, finding 15) are both inferred, downstream, in Rust.

**Deferred to Phase 6+:** incremental rebuild (source-hash keyed) and
SQLite/RocksDB-backed storage (`query` currently reads `graph.json`
directly). Canonical-node deduplication for `conref`/`conkeyref`-reused
content is done, see Phase 6+ below.

**Found and fixed for real-dataset usability (post-`v0.1.0`):**
`infer_related_to` (`relations.rs`) was an unconditional O(n²) sweep
over every topic pair, flagged in its own doc comment as "fine for the
corpus sizes this scaffold targets; revisit... if that stops being
true" — exactly the kind of thing meant to be revisited once tested
against real corpus sizes, not before. Rewritten around a `product ->
topic indices` bucket index (built once, O(n)) so a topic is only ever
compared against others that actually share a `product` value, not
every topic in the corpus regardless of overlap; the apply phase also
moved from an O(edges × n) linear `find()` per edge to an O(edges)
id-index lookup. Verified empirically, not just reasoned about: a
synthetic 5,000-topic corpus with a realistic tag-like `product` spread
(300 distinct values) completes the whole `build` — inference, writing
5,000 concept files, the RAG index, validation — in ~1.1s; the genuine
degenerate worst case (all 5,000 topics sharing one `product` value,
producing 24,995,000 edges — a corpus where `product` carries no
distinguishing information at all) takes ~2m10s, which is inherent to
that case's actual output size, not a complexity regression. Verified
correct with a dedicated bucketing test (two separate product groups
plus an unrelated topic — every within-group pair found, zero
cross-group edges) alongside the existing suite, all deterministic
(`BTreeMap`/`BTreeSet` throughout, matching the old loop's edge order
exactly).

### Phase 3 — MCP server

`dita2graph-mcp` implements the JSON-RPC-over-stdio pattern with the
full tool set — `search_topics`, `search_content`, `find_related_topics`,
`explain_task`, `trace_dependencies`, `analyze_impact`,
`generate_summary`, `validate_bundle` — verified end to end over real
stdin/stdout against a live-built bundle. Takes a bundle root directly
or via `--config <mcp-server.toml>` (written by `dita2graph-core build
--mcp true`).

**Deferred:** §5.1's Resources (`resources/list`/`resources/read`,
`dita://topics` etc.) are not implemented at all — every capability
they'd expose is already reachable through the tool set above, so this
hasn't blocked real use, but the gap is real and documented, not
assumed away.

**Found and fixed for real-dataset usability (post-`v0.1.0`):** every
`tools/call` was independently reopening and reparsing `graph.json`,
plus (per whichever tool) `rag/chunks.jsonl` and each concept file it
touched, from scratch — harmless against a small fixture, but a real
agent session against a real, sizeable bundle issues many tool calls
against the same data, and `search_topics` alone reads every topic's
concept file on every call just to display titles. `BundleCache`
(`mcp/dita2graph-mcp/src/bundle.rs`) now holds one `BundleReader` for
the server's process lifetime, reopening only when `graph.json`'s mtime
shows the bundle actually changed (a mid-session rebuild), and falling
back to the last-loaded bundle rather than failing a call outright if
that reopen catches the rebuild mid-write. `BundleReader` itself caches
per-id concept reads and the parsed `rag/chunks.jsonl` (`Rc`-shared, not
deep-cloned, on every cache hit) for its own lifetime. Verified with
unit tests proving the caches are real (editing a file on disk after
the first read doesn't change what a second call on the same reader
returns) and that reload/fallback behavior is correct, plus a live
multi-call session against a DITA-OT-built bundle.

A code review after the fact caught a follow-up gap in the staleness
check itself, since fixed: `dita2graph-core build` writes `graph.json`
(via `write_bundle`, last, after every concept file) and rewrites
`rag/chunks.jsonl` (via `write_rag_index`) as two separate steps, so
fingerprinting `graph.json`'s mtime alone meant a `get()` landing in
that window could permanently cache a stale-or-not-yet-rewritten rag
index until the *next* rebuild changed `graph.json` again.
`BundleCache` now fingerprints both files' mtimes (still two cheap
`stat()`s, not a corpus-wide scan) — verified with a regression test
that reproduces the exact two-step-write race, plus a test proving a
bundle with no `rag/` at all still caches normally rather than being
treated as permanently stale.

### Phase 4 — Gradle integration + CI hardening

`gradle-build/`'s Kotlin DSL harness runs the entire pipeline for real:
`buildKnowledgeGraph`, the public/internal DITAVAL split, and dedicated
fixture tasks for nested maps, map composition, and relation inference.
CI (`.github/workflows/`) runs the Rust and Java unit-test suites plus
a full live-DITA-OT integration run on every push, including a
deliberately-broken fixture that proves the validation gate actually
fails the build before extraction runs, not just in theory.

### Phase 5 — Security, docs, and public `v0.1.0` release

Licensing decided and shipped (dual MIT OR Apache-2.0, uniform across
Rust/Java/Gradle). Secret-leakage detection is build-breaking, not a
warning, and covers both `okf/` and `rag/`. The public/internal DITAVAL
split is a demonstrated pattern with a real fixture topic, not a no-op.
README and this roadmap document the actual, current state. `v0.1.0` is
tagged.

**Release automation for future versions:** `.github/workflows/tag.yml`
(`workflow_dispatch` from `main` — validates the requested version
against `Cargo.toml` before tagging) triggers
`.github/workflows/release.yml` on the resulting tag push: re-runs the
unit-test suites as a release gate, builds the Rust binaries and the
DITA-OT plugin zip (reusing `gradle-build/`'s own tested
`zipDita2GraphPlugin`/`installDita2Graph` tasks to prove the released
zip actually installs into a live DITA-OT 4.4, not just that `zip`
produced a file), and publishes a GitHub Release with all three
artifacts attached.

### Phase 6+ — Extended capabilities (post-MVP, ongoing)

Not a single phase but a backlog, picked up item by item. Current
state, most-complete first:

1. **Canonical-node deduplication** for `conref`/`conkeyref`-reused
   content — ✅ done (topic-level granularity), verified against a live
   DITA-OT 4.4 run. A reusing topic's OKF body and RAG chunk text now
   exclude spans pulled in via `conref`/`conkeyref` (detected the same
   way `generated-from` already was, via `xtrf` mismatches); that text
   continues to live exactly once, in its source topic, with the
   existing `generated-from` edge as the pointer. See
   [`docs/dev/canonical-node-dedup-spec.md`](docs/dev/canonical-node-dedup-spec.md)
   for the design, edge cases, and exit-criteria evidence.
2. **Hybrid graph + RAG architecture** — nearly done. `rag/chunks.jsonl`
   extraction (same single pass as `okf/`), `search_content`'s
   graph-narrowed and keyword-frequency-ranked query routing, and
   `analyze_impact`'s reverse traversal with text excerpts are all
   implemented and verified. Only node-level embeddings (semantic
   similarity ranking, as opposed to keyword overlap) remain — a
   heavier change to the OKF bundle format itself, listed as a
   direction under consideration, not a committed design.
3. **Incremental rebuild** (source-hash keyed) and **SQLite/RocksDB
   storage** for the query index.
4. **Full `<navref>` map composition** — would need this plugin to
   independently parse and merge referenced navigation maps outside
   DITA-OT's own pipeline, losing keyref/conref resolution and DITAVAL
   filtering for that content specifically. A materially larger,
   riskier undertaking than the `mapref`/`anchorref` support already
   shipped; the gap is at least visible now (`DITA2GRAPH060W`), not
   silent.
5. **MCP Resources** (§5.1) and **HTTP transport** (§6.3) for the MCP
   server — stdio + tools cover real use today; these are additive.
6. **Other extended capabilities** (§13.2, least scoped): multi-map/
   multi-product graph federation, graph versioning/diffing across doc
   releases, and a rendered-output (PDF/HTML5) variant that annotates
   pages with links back into the graph.

Each item gets its own scoped follow-up spec and exit criterion before
work starts, the same discipline that shaped everything above it.
