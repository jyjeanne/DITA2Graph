# Spec: canonical-node deduplication for `conref`/`conkeyref` reuse

Status: **scoped, not started**. Roadmap §6+ item 2
([`Roadmap.md`](../../Roadmap.md)). Written before implementation per the
roadmap's own rule ("each item gets its own scoped follow-up spec and
exit criterion before work starts").

## Problem

`conref`/`conkeyref` reuse is the defining feature of real DITA
corpora — shared warnings, legal boilerplate, glossary entries, and
common procedure steps single-sourced across dozens or hundreds of
topics. `generated-from` already records *where* reused content came
from (finding 15, `docs/dev/phase-0-findings.md`), derived
deterministically from DITA-OT's own `xtrf` source-trace attributes —
but the reusing topic's OKF body still contains the pulled-in text
inline, identical to DITA-OT's own resolved output
(`docs/plugin-specification.md` §3.3, "Deduplication & reuse
tracking"). On a real dataset this means:

- Every MCP tool that reads body text — `search_content`,
  `analyze_impact`, `explain_task`, `generate_summary` — sees the same
  reused fragment duplicated across every topic that reuses it.
  `search_content`'s ranking is keyword-frequency-based today (§13.1),
  so heavily-reused boilerplate (a shared warning note, say) can
  out-rank genuinely distinctive content purely because it's
  physically copied into more topics.
- The bundle and `rag/chunks.jsonl` both grow with copies of
  identical text instead of one canonical copy plus pointers.
- `analyze_impact`'s reverse traversal already has the right edge
  (`generated-from`) to reason about "which topics does editing this
  shared content affect" — but has no way to know today's body text
  in each of those topics is literally the same paragraph, not just
  topically related.

## What already exists (don't re-derive)

- `generated-from` edges: one per (reusing topic, source topic) pair,
  Java side, `DitaModelExtractor.java:280-307`. Detected by walking
  every descendant element of a topic's resolved body and comparing
  its `xtrf` attribute against the topic's own source file; a mismatch
  means that element was pulled in via `conref`/`conkeyref` (a plain
  `keyref`-resolved `<keyword>`/`<ph>` keeps its own `xtrf` — confirmed
  directly, finding 15, not assumed).
- `Link.java` / `model.rs:155` (`Relation::GeneratedFrom`): the edge
  type, already flows through `okf.rs`'s bundle writer as a markdown
  link, no format change needed there.
- Fixture: `sample-docs-relations/` — `topics/shared-content.dita`
  (topicref'd in `user-guide.ditamap`, so it's already a first-class
  graph node, not an orphan) has one `<p id="warning-note">`;
  `topics/reuser.dita` pulls it in via `conref` plus has its own
  unreused paragraph. `DitaModelExtractorTest.
  generatedFromIsExtractedFromXtrfMismatches` covers the edge; it does
  **not** yet assert anything about body text content.

## What "canonical" means here — granularity decision

**Scope: topic-level, matching `generated-from`'s existing
granularity.** A reusing topic's OKF body/RAG-chunk text excludes any
span whose `xtrf` points at a different source file; that text
continues to live exactly once, in the source topic's own body, and
the existing `generated-from` edge is the pointer from every reuser
back to it.

**Explicitly not in scope: sub-topic/element-level fragment nodes**
(e.g. a standalone node per `<p id="warning-note">`, independent of
its containing topic). `xtrf` resolves to a `file:` URI matching a
`JobFile`'s `src` — whole-file granularity, confirmed directly against
live DITA-OT 4.4 output (finding 15) — it does **not** carry the
conref'd element's own id or an in-file offset. Getting to
element-level canonical nodes would mean either parsing original
`conref`/`conkeyref` attribute values *before* DITA-OT resolves them
away (a second, independent extraction path DITA-OT doesn't hand you
for free) or minting synthetic ids from position/content hashing
(fragile across edits). That's a materially bigger, separate piece of
work with its own spec — the same judgment call the roadmap already
made for `<navref>` (§6+ item 4). Topic-level dedup captures the
dominant real-world case (a shared library topic conref'd wholesale or
by paragraph into many reusing topics, per `sample-docs-relations`'
own fixture shape) without it.

## Design

### Java extractor (`DitaModelExtractor.java`)

`bodyText()` (line 632) currently does one `getTextContent()` call
over the whole body element — no per-element walk. Rewrite it to walk
the body's element tree top-down (reusing the same traversal shape as
the existing `generated-from` loop at line 292) and, for each
block-level element:

- If its own `xtrf` differs from the topic's own source file's `src`
  (the same `ditaSrcToPath` lookup `generated-from` already uses),
  exclude its full text content and do not descend into its children
  (DITA-OT replaces a `conref`'d/`conkeyref`'d element *wholesale*
  with the referenced one, inheriting that element's `xtrf` — finding
  15 — so a foreign `xtrf` on a block element means everything under
  it is foreign too; descending separately would either double-count
  or, worse, partially include a subtree DITA-OT itself treats as one
  resolved unit).
- Otherwise, include its own direct text and recurse into children.

This produces the same "cleaned, whitespace-collapsed text" contract
`bodyText()` already promises, just with foreign spans removed instead
of included. No change to the `generated-from` detection loop itself —
it can stay as-is, or be merged with this walk in the implementation
(single tree traversal instead of two) as an internal efficiency
choice, not a spec requirement.

### Rust core — no format change needed

`NormalizedTopic.body` (`model.rs:73`) stays `Option<String>`.
`okf.rs`'s bundle writer and `rag.rs`'s chunk extraction both already
derive from this one field (`rag.rs:1-13`, "both derived from the same
normalized-model slice... not a second parse") — deduplication at the
Java extraction step flows through to the OKF bundle body *and*
`rag/chunks.jsonl` for free. No new node type, no new relation, no new
frontmatter key.

### Edge cases

| Case | Behavior |
|---|---|
| Topic's entire body is one conref'd block | Resulting `body` is empty — `Option<String>` already models this (`shortdesc`/`body` are both optional today); no crash, no special-case needed. |
| Reused element nested inside the reusing topic's own non-reused structure (e.g. a `<step>` with a conref'd `<info>` but authored `<cmd>`) | Handled by the top-down walk: the `<info>` subtree is excluded, sibling `<cmd>` text is kept, because exclusion is decided per block element, not per topic. |
| Inline-level reuse mid-sentence (`<ph conref="...">` inside authored prose) | Best-effort only in v1: if excluding a non-block element would leave a dangling sentence fragment, leave it in rather than mangle prose. Block-level reuse (`<p>`, `<step>`, `<note>`, `<li>`, …) is the dominant real-world pattern (matches `sample-docs-relations`'s own fixture) and the one this spec commits to solving correctly. |
| The source topic itself | Never affected — its own elements' `xtrf` matches its own file by definition, so nothing is excluded from the topic that *authors* the shared content. |
| Conref-only library topic never `topicref`'d into any map | Already produces its own `TopicNode` today (`DitaModelExtractor` iterates every `format="dita"` `JobFile` from `job.xml`, not just ones reachable from the map tree) — no change needed for it to exist as the canonical target of the pointer. |

### Non-goals (explicit)

- Element/fragment-level canonical nodes (see granularity decision
  above) — future work, own spec.
- Any change to `contains`/`requires`/`references`/`related-to`/
  `applies-to` extraction or inference — only body/RAG-chunk *text*
  extraction changes.
- Any OKF bundle format or frontmatter schema change.
- Cross-DITAVAL-variant dedup (e.g. collapsing the same fragment
  across a public and internal build of the same source) — out of
  scope, each bundle build is independent per §5's DITAVAL split.

## Exit criteria

1. **Unit** (`DitaModelExtractorTest.java`): extend
   `generatedFromIsExtractedFromXtrfMismatches` (or add a sibling
   test) to assert `reuser`'s extracted `body` contains "Own content
   not reused." and does **not** contain "Reusable content." /
   "Another reused paragraph via conkeyref." — while the existing
   `generated-from` edge assertion continues to pass unchanged.
2. **Fixture, live DITA-OT 4.4**: extend `sample-docs-relations/`
   (reusing the existing `shared-content.dita`/`reuser.dita` pair —
   no new fixture directory needed) and add/extend a Gradle fixture
   task alongside the existing relation-inference task
   (`gradle-build/`, Phase 4 pattern) that builds the bundle and
   asserts on the rendered `okf/topics/reuser.md` body text directly,
   not just the Java unit test's in-memory model.
3. **RAG**: `rag/chunks.jsonl`'s `reuser` chunk's `text` field matches
   the same deduplicated body — no separate code path to verify twice,
   but assert it explicitly so a future refactor that decouples
   `rag.rs` from `NormalizedTopic.body` doesn't silently regress this.
4. **Bundle conformance**: `okf_validator::validate_bundle` still
   passes on the resulting bundle (dropping text from a concept body
   doesn't touch frontmatter, links, or reachability — this should be
   a non-event, but it's cheap to assert).
5. **CI**: existing full-pipeline live-DITA-OT integration run
   (Phase 4) continues to pass with the extended fixture folded in,
   not a separate opt-in check.

## Rollout

No CLI flag, no config toggle — this corrects extraction to match
what `generated-from` already implies (reused content has one
authoritative source), so it should be the new unconditional
behavior, not an opt-in. Document the change in the next release's
notes since it changes bundle body content for any dataset that uses
`conref`/`conkeyref` (i.e., most real ones) — not a breaking format
change, but a visible content change worth calling out.
