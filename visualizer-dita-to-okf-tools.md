# Visualizing DITA2Graph's OKF Output — Tool Study

**Subject:** how the `okf/` bundle this repo already produces (§4 of
`docs/plugin-specification.md`) can be *browsed and graph-visualized*,
not just queried through MCP.
**Scope:** Obsidian-as-vault ("LLM wiki" pattern), OKF-native
visualizers, VS Code wiki-graph tools, and generic node/edge graph
tools driven by `okf/graph.json`.
**Status:** Study only — nothing in this document has been implemented.
No code changes accompany it.

---

## 1. Executive summary

DITA2Graph already writes exactly the artifact this kind of tool wants:
a directory of UTF-8 markdown files, each with a YAML frontmatter block
and a body full of relative markdown links (`docs/plugin-specification.md`
§4.4; real example at `okf/topics/installing-product.md`). That's the
literal definition of an [OKF](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundle, and it also happens to be the literal definition of an
[Obsidian](https://obsidian.md) vault. **No conversion step exists between
"OKF bundle" and "thing a markdown-wiki tool can open" — they're the
same directory.**

The practical finding: **pointing Obsidian at `okf/` works today, with
zero code changes**, and gets you a force-directed graph, backlinks, and
full-text search over the bundle for free. What it does *not* get you,
out of the box, is the thing that makes this graph different from a
generic wiki — DITA2Graph's **typed** relation taxonomy (`contains` /
`requires` / `references` / `applies-to` / `related-to` /
`generated-from`, §4.3). Obsidian's core graph view draws one kind of
edge; recovering the types needs either a companion plugin reading the
`relations` frontmatter extension, or a purpose-built OKF viewer that
already understands it.

That purpose-built alternative exists: the OKF spec's own reference
implementation ships a `visualize` subcommand that renders any bundle
as a single self-contained interactive HTML file (Cytoscape.js-based,
force-directed, color-by-`type`, click-through detail panel) — no
backend, nothing to install for the person viewing it. It's the closest
thing to an "official" OKF viewer that exists, and it's a better fit
than Obsidian for a one-shot, shareable, no-account-needed view (e.g.
attaching it to a PR or a release), while Obsidian is the better fit for
a technical writer who wants to live in the graph day to day.

Recommendation is in §8: don't build a bespoke viewer now; document the
Obsidian quickstart (it costs nothing, §3), and treat an OKF-native
`visualize` HTML export as a plausible, cheap Phase 6+ roadmap item
because DITA2Graph already has everything that subcommand would need
(`okf/graph.json` gives it the node/edge list for free — the reference
implementation has to derive that from parsing markdown links itself).

---

## 2. What's actually being visualized

Grounding this study in what the repo produces today, not a hypothetical
format (`docs/plugin-specification.md` §4.4, `README.md`):

```
okf/
├── maps/
│   └── user-guide.md         # type: DITA Map — relations.contains mirrors topicref hierarchy
├── topics/
│   ├── installing-product.md # type: Task — relations.requires: [configuration]
│   ├── configuration.md      # type: Concept
│   └── ...
└── graph.json                 # derived, flattened {nodes:[{id,type}], edges:[{from,to,relation}]}
```

Each concept file:

```markdown
---
type: Task
title: Installing Product
description: Steps to install the product in a production environment.
resource: topics/installing-product.dita
tags: [admin, enterprise, install-task]
generated: { by: dita2graph-core/0.1.0, at: 2026-08-03T00:00:00Z }
relations:
  requires: [configuration]
---

# Summary
...
# Content
...
# References
- [Installing Product: Prerequisites](installing-product-prereqs.md)
# Requires
- [Configuration Overview](configuration.md)
```

Three things matter for tool compatibility, all confirmed against
`docs/plugin-specification.md` §4.1/§4.4 rather than assumed:

1. **Links are standard markdown links** (`[text](path.md)`), not
   Obsidian's `[[wikilink]]` syntax. This is good news for portability —
   every tool surveyed below reads plain markdown links — but it means
   any tool that *only* understands `[[wikilinks]]` (a real subset of
   the "PKM tool" space, e.g. plain TiddlyWiki or Roam-style tools) needs
   a conversion pass DITA2Graph doesn't produce today.
2. **The typed relation taxonomy lives in two places at once for some
   edges**: `requires`/`applies-to`/`contains` are captured *both* as a
   `relations:` frontmatter block *and* rendered as a plain link under a
   `# Requires`/`# References` heading in the body (see the example
   above — `relations.requires: [configuration]` and the `# Requires`
   section both exist). `references`/`related-to`/`generated-from` rely
   on the body link alone. Practically: **a tool that only parses
   markdown links (no frontmatter awareness) still recovers every edge**,
   it just can't recover the *type* of edge without also reading
   `relations:` or the section heading it's under.
3. **`graph.json` is explicitly "derived, not authoritative"** (§4.4) —
   a flattened `{id, type}` / `{from, to, relation}` view that exists
   for tooling convenience. It is the natural input for any generic
   graph-analytics tool (§6) that doesn't want to parse markdown at all.

---

## 3. Obsidian as an OKF viewer ("LLM wiki" pattern)

This is the pairing the task asked about, so it gets the most detail.

### 3.1 Why it's close to a direct fit

Obsidian's data model *is* "a folder of markdown files with YAML
frontmatter, linked by relative paths, watched live on disk" — which is
also the OKF spec's entire pitch (the spec's own words: "if you can
`cat` a file, you can read OKF"). Concretely, opening `okf/` (or a whole
built bundle directory) as an Obsidian vault (**File → Open folder as
vault**) gets you, with no import step and no plugin:

- **Graph view** (global + per-note local graph) built from the same
  markdown links `okf/graph.json` also encodes — Obsidian parses these
  itself; it doesn't need `graph.json` at all.
- **Backlinks** per note — "what points at this concept" — which is a
  cheap approximation of `find_related_topics`/`analyze_impact`'s
  reverse-edge queries, without going through the MCP server.
- **Properties pane** reading the YAML frontmatter natively — `type`,
  `title`, `tags`, `resource`, `generated`, `relations` all show up
  as structured properties, not raw text.
- **Tag pane + tag-based graph coloring** — DITA2Graph maps
  `audience`/`platform`/`product`/`otherprops` straight onto OKF's
  `tags:` field (§4.1's mapping table), and Obsidian's tag pane and
  graph-color-groups both key off exactly that frontmatter field with
  zero adaptation.
- **Full-text search** over the `# Content` body — a manual, in-editor
  alternative to `search_content`.
- **Live updates** — Obsidian watches the vault folder; re-running
  `buildKnowledgeGraph` and regenerating `okf/` shows up in the open
  vault without reopening it, which fits an iterative writer/build loop.

One link-format setting matters: Obsidian defaults new vaults to
wikilink-style *new* links, but it has always read and resolved plain
markdown links like DITA2Graph produces — this is a link-format
*preference* for links Obsidian itself creates, not a compatibility gate
on links it opens. No setting change is required to open an `okf/`
bundle and see its graph; the wikilink toggle only matters if a writer
starts hand-authoring new links from inside the vault (not a supported
workflow here — see §3.3).

### 3.2 What Obsidian does *not* recover on its own

- **Edge types.** Core Graph View draws one undirected-looking edge
  style for every link; it cannot distinguish a `contains` edge (map →
  topic) from a `requires` edge (task → concept) from a plain
  `references` cross-reference. All three render identically. Given
  §4.3's whole point is that DITA relations are typed and directional,
  this is the single biggest gap between "opened it in Obsidian" and
  "got the actual graph DITA2Graph modeled."
- **Directionality.** Markdown links are directed in the source, but
  Graph View renders them as undirected. "What does X require" and
  "what requires X" look the same in the picture; you still have to
  read the note to tell forward from reverse (exactly the distinction
  `trace_dependencies` vs `analyze_impact` exists to make explicit).
- **`graph.json` itself.** Nothing in Obsidian reads it — it's inert
  alongside the vault. Not a problem (Obsidian re-derives the same
  graph from the links), just worth knowing `graph.json` buys nothing
  extra for an Obsidian-based workflow; its payoff is entirely for the
  generic tools in §6.

### 3.3 Recommended companion plugins, mapped to what they'd close

None of these are required to get *a* graph — only to get the typed one:

| Plugin | Closes which gap | Notes |
|---|---|---|
| **Dataview** (community) | Typed relations, queryable | A query like `LIST relations.requires FROM #install-task` reads the frontmatter extension directly and can render per-type tables/lists DITA2Graph's `relations:` block was designed to carry. |
| **Bases** (core, Obsidian 1.9+) | Same, no plugin install | Native as of early 2026 — a filtered table/card view over frontmatter properties (`type`, `tags`, `relations`) without installing a community plugin. Weaker query language than Dataview but zero extra trust surface. |
| **Breadcrumbs** (community) | `contains` hierarchy specifically | Purpose-built for parent/child structural relations — a closer semantic match to the map→topic `contains` edge than the generic graph view, and can render it as a literal breadcrumb trail or hierarchy tree. |
| **Connections** (community) | Typed, directional edges generally | Lets a vault define named relationship types and visualize them in a dedicated pane; closest community-plugin match to §4.3's taxonomy as a whole, at the cost of a mapping step (frontmatter `relations.*` keys → the plugin's own connection-type config). |

None of these change what's on disk — they're all read-only lenses over
the same `relations:` frontmatter DITA2Graph already writes. No
DITA2Graph code changes are implied by using any of them.

### 3.4 Friction points worth flagging before recommending this workflow

- **The bundle is generated, read-only content.** `okf/` is a build
  artifact of `buildKnowledgeGraph` — editing a concept file inside
  Obsidian edits a copy that the next build silently overwrites.
  Authoring stays in DITA source; Obsidian here is a *read* surface,
  same posture as opening resolved HTML output in a browser. Worth
  saying explicitly to a writer before they invest annotations in the
  vault.
- **`okf/` vs the whole bundle root.** Opening `gradle-build/build/dita2graph/`
  itself (rather than just `okf/`) as the vault also pulls `rag/`
  (`chunks.jsonl`, `metadata.json` — not markdown, mostly noise for a
  graph view) and, if built with `--args.dita2graph.mcp=true`,
  `mcp/mcp-server.toml` into the same vault. Point Obsidian at `okf/`
  specifically, not the parent bundle directory.
- **DITAVAL public/internal split (§6.1).** `buildKnowledgeGraphPublic`/
  `buildKnowledgeGraphInternal` produce two separate `okf/` trees. That
  maps naturally onto **two separate vaults**, which is actually a
  feature here — a writer doing pre-publish review (README's "Pre-publish
  review" use case) can visually diff what's in one graph and not the
  other, which is a stronger check than `diff`-ing file lists.
- **Node coloring by DITA topic `type`** (Task/Concept/Reference/Glossary
  Entry/DITA Map) isn't automatic — Graph View's color groups are
  regex-over-path or tag-based, not frontmatter-property-based. Getting
  the same "color nodes by type" effect Google's reference-agent
  visualizer does out of the box (§4 below) needs either mirroring
  `type` into a tag at write time (a DITA2Graph-side change, not
  attempted here) or a folder convention Obsidian's path-regex grouping
  can key off — `okf/` already separates `maps/` from `topics/`, which
  is one free axis, but doesn't separate Task from Concept from
  Reference within `topics/`.

### 3.5 Net verdict

Worth documenting as a zero-cost quickstart addition (e.g. a short
section in `docs/tutorial.md` next to the existing MCP walkthrough:
"Part 3 — browse the bundle visually"). It's genuinely the least
friction option of everything surveyed here, and it directly matches
what "study how [...] OKF graph [...] could be visualised with a tool
like Obsidian with his Vault like llm wiki" was asking about. Its one
real shortfall — typed/directed edges — is exactly the thing
DITA2Graph's own MCP tools (`find_related_topics`, `trace_dependencies`,
`analyze_impact`) already answer precisely; Obsidian complements those
tools for exploration, it doesn't replace them for anything that needs
the typed graph.

---

## 4. OKF-native visualizers

### 4.1 The spec's own reference viewer

The OKF spec's reference implementation
([`GoogleCloudPlatform/knowledge-catalog`](https://github.com/GoogleCloudPlatform/knowledge-catalog),
`okf/` directory) ships a `visualize` subcommand on its `reference_agent`
CLI:

```
.venv/bin/python -m reference_agent visualize --bundle ./bundles/<name> [--out viz.html] [--name "..."]
```

It renders the whole bundle as **one self-contained HTML file** — no
server, no install needed by whoever opens it, shareable as a plain
file or hosted statically. Under the hood it's Cytoscape.js (graph) +
`marked` (rendering the markdown body), both pulled from a CDN, with
the bundle's data embedded inline as JSON. Features directly relevant
here: force-directed layout with several alternates (concentric,
breadth-first, circle, grid), **node coloring by `type`** (the thing
§3.4 flagged Obsidian can't do out of the box), a detail panel showing
frontmatter + rendered body + backlinks ("cited by") for the selected
node, search over title/id/tags, and type filtering.

This is the closest thing to an "official" OKF viewer that exists. It
would need to be pointed at a DITA2Graph-produced `okf/` bundle
directly — nothing in its design is BigQuery- or Google-Cloud-specific,
despite the reference implementation's origin (it was built to
visualize OKF concepts generated from BigQuery tables/datasets, but the
`visualize` subcommand itself just walks a bundle directory, exactly
like every tool in this study does). Not vendored or tested against
this repo's bundles as part of this study — a natural first validation
step if this direction is pursued (§8).

### 4.2 Third-party OKF tooling built on top of the spec

[`scaccogatto/okf-skills`](https://github.com/scaccogatto/okf-skills) is
a Claude Code plugin (also distributed as agent skills for other
agents, and as a GitHub Action) offering `/okf:okf` (author/maintain),
`/okf:validate` (spec conformance), and `/okf:visualize` — the last one
also producing a self-contained interactive HTML graph, additionally
computing "trust tier" and "staleness" indicators at render time from
the `sources`/`generated`/`verified`/`stale_after` frontmatter families
(§4.1's table — DITA2Graph doesn't currently populate `sources`/`status`/
`stale_after`, so those indicators would render as unknown/absent for a
DITA2Graph bundle today, not broken, just uninformative).

Notable as evidence the ecosystem around OKF visualization is small but
real and converging on the same shape (single static HTML, node-per-
concept, color-by-type, click for detail) — not as something to adopt
directly without further evaluation, since both this and §4.1 are
early-stage, unaudited third-party/reference code, not a stable
dependency.

---

## 5. VS Code wiki-graph tools — a synergy worth naming

This repo already has a VS Code-adjacent dependency: `validate_live`
vendors [DitaCraft](https://github.com/jyjeanne/ditacraft)'s LSP, whose
primary UI is a VS Code extension (`README.md`, "Live validation via
DitaCraft's LSP"). A technical writer already living in VS Code for live
DITA validation is one extension install away from a wiki-graph view of
the same content, without leaving the editor:

- **[Foam](https://foambubble.github.io/foam/)** — a VS Code extension
  (not a proprietary app/vault format like Obsidian) giving a graph
  view, backlinks, and tag pane over a plain folder of markdown files,
  explicitly supporting standard `[text](path.md)` links as well as
  wikilinks. Git-friendly by design (it assumes the notes live in a
  repo). The natural "same editor as DitaCraft" pairing.
- **Dendron** — also VS Code-based, but its hierarchy model is
  dot-delimited filenames (`topic.subtopic.md`) rather than directory
  nesting, and it expects its own frontmatter schema conventions. More
  adaptation needed than Foam to sit cleanly over an `okf/` bundle as-is
  — noted for completeness, not recommended over Foam.

Not evaluated hands-on in this study; flagged because the "already in
this editor for another reason" argument is unusually strong here,
stronger than it would be for an arbitrary repo.

---

## 6. Generic graph.json consumers

For anything that wants graph *analysis* more than graph *browsing* —
centrality, clustering, large-doc-set scale — `okf/graph.json`'s
flattened `{nodes:[{id,type}], edges:[{from,to,relation}]}` shape
(§4.4) is a deliberately simple, tool-agnostic handoff point. None of
these read the markdown bodies or frontmatter; they only need the
already-typed edge list `graph.json` provides, which makes them a
cleaner fit for typed-edge analysis than the markdown-link-only tools
above.

| Tool | Fit |
|---|---|
| **Cytoscape.js** (embed) / **Cytoscape Desktop** (standalone) | Same library the OKF reference viewer uses (§4.1); best open-source option for graph-theory-grade analysis (centrality, shortest path) alongside visualization, at no cost. Would need a small adapter mapping `graph.json`'s shape to Cytoscape's `elements` format. |
| **Gephi** | The open-source standard for exploring/styling large networks; consumes GEXF/GraphML/CSV, so `graph.json` needs a conversion step first. Better suited to a doc set large enough that force-directed layout in a browser starts to strain. |
| **vis-network** | Good default for an embeddable, interactive "diagram-like" graph (drag nodes, physics) rather than research-grade analysis — a reasonable base if DITA2Graph ever ships its own lightweight embedded viewer (§8's stretch option). |
| **Cosmograph** | GPU/WebGL-accelerated, built for graphs the size this project isn't at yet (thousands+ nodes) — noted for scale headroom, not a near-term need against `sample-docs/`-sized fixtures. |
| **Neo4j** (Browser/Bloom) | Overkill for a single bundle, but plausible if DITA2Graph's graph ever needs to be queried *alongside* other systems' knowledge graphs (Cypher joins across sources) rather than browsed standalone — a "when this stops being a single-repo concern" option, not a near-term recommendation. |

None of these were run against a real DITA2Graph `graph.json` as part of
this study — evaluation was desk research against each tool's own docs
and current (2026) comparisons, not a hands-on trial.

---

## 7. Comparison

| | Obsidian | OKF-native viewer (§4) | Foam (VS Code) | Cytoscape.js/Gephi/etc. (§6) |
|---|---|---|---|---|
| Setup cost against an existing `okf/` bundle | None — open folder as vault | Run a Python CLI against the bundle | Install a VS Code extension | Write an adapter for `graph.json`'s shape |
| Typed/directed edges shown natively | No (needs a plugin, §3.3) | Yes (color by `type`; detail panel) | No | Yes — `graph.json` already carries `relation` per edge |
| Reads `graph.json` | No (re-derives from links) | No (walks markdown itself) | No | Yes — this is its native input |
| Shareable as a single file, no viewer install | No (needs Obsidian installed) | Yes — one HTML file | No | Depends on tool |
| Best fit | Day-to-day writer exploration, backlinks, search | One-shot shareable snapshot, PR/release artifact | Writer already living in VS Code for DitaCraft | Large-scale analysis, not routine browsing |
| Change required in DITA2Graph to use today | None | None (bundle format already compatible) | None | Small adapter script |

---

## 8. Recommendation

1. **Document the Obsidian quickstart now** — it's genuinely zero-cost
   (§3.5) and directly answers what this study was asked to investigate.
   A short addition to `docs/tutorial.md`, alongside the existing MCP
   walkthrough, pointing a reader at `okf/` with the two caveats that
   matter most in practice: it's read-only content (§3.4), and edge
   *types* aren't visible without a companion plugin (§3.2/§3.3).
2. **Don't build a bespoke DITA2Graph viewer yet.** §4.1's OKF-spec
   reference `visualize` subcommand already does most of what a
   bespoke tool would (color-by-type, detail panel, shareable single
   file) and DITA2Graph's `graph.json` is strictly *more* than what it
   needs (it already carries typed, directed edges the reference tool
   otherwise has to infer from markdown links). Validating that
   subcommand against a real DITA2Graph bundle is a cheap first step
   before considering porting the idea in-repo.
3. **Treat an in-repo `visualize` export as a plausible, narrow Phase
   6+ item, not urgent work** — a static HTML export (vis-network or
   Cytoscape.js, per §6) driven directly by `okf/graph.json` would be
   small (the hard part, typed/directed edges, is already solved data,
   not new inference work) and would give DITA2Graph something to
   attach to a PR or release the way `gradle-build/build/dita2graph/okf/`
   currently can't be shown without either an MCP client or a manual
   Obsidian vault open. Not scoped further here — this study stops at
   "worth a Roadmap line," not a design.
4. **No change needed to `okf.rs`/`graph.json`'s schema** for any option
   in this study — every tool surveyed either reads the markdown bundle
   directly (Obsidian, Foam, OKF-native viewers) or reads
   `graph.json`'s existing shape as-is (§6). This study found no gap in
   what DITA2Graph already writes; the gap is entirely in *which viewer
   a person reaches for*, not in the data.

---

## 9. What this study didn't cover

- No tool above was run hands-on against a real, built
  `gradle-build/build/dita2graph/okf/` bundle from this repo — every
  finding is grounded in each tool's own documentation plus the exact
  bundle shape `docs/plugin-specification.md` §4.4 documents, not a live
  trial. Before acting on §8.1 or §8.2, open a real bundle in Obsidian
  and run the OKF reference `visualize` subcommand against one, to
  confirm the desk-research findings above hold against actual output
  (particularly whether Obsidian's link resolution handles the
  `../topics/`-style relative paths in `okf/maps/user-guide.md` cleanly
  when opened from `okf/` as vault root — expected to work per Obsidian's
  standard relative-markdown-link resolution, but not verified here).
- Publishing a public-facing browsable graph (e.g. via
  [Quartz](https://quartz.jzhao.xyz/), which turns an Obsidian-style
  vault into a static "digital garden" website) wasn't evaluated in
  depth — flagged only as a plausible complement to the DITAVAL
  public/internal split (§6.1) if DITA2Graph ever wants a published,
  browsable public doc-set graph, not analyzed further here.
- Accessibility, licensing, and offline/air-gapped constraints of each
  third-party tool (Obsidian is closed-source freeware with a paid sync
  tier; the OKF reference viewer's CDN-loaded Cytoscape.js/`marked`
  would need vendoring for an air-gapped environment) weren't audited —
  worth doing before any of these is put in front of an enterprise
  DITA2Graph user with those constraints.
