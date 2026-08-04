# sample-docs-mapref

A fixture for all three map-composition mechanisms
(`docs/plugin-specification.md` §3.3, `docs/dev/phase-0-findings.md`
findings 14 and 16) — `<mapref>`, `anchorref`, and `<navref>` — checked
in after actually testing each against a live DITA-OT 4.4, not assumed
from the spec text alone.

`submap.ditamap` is pulled into `user-guide.ditamap` via `<mapref
href="submap.ditamap"/>`, and its root `<map>` carries
`anchorref="insert-point"`, referencing the `<anchor id="insert-point"/>`
nested inside `topics/intro.dita`'s `<topicref>` in the base map.
`user-guide.ditamap` also carries a `<navref mapref="nav.ditamap"/>`,
pointing at `nav.ditamap`/`topics/nav-topic.dita`.

Confirmed directly (not assumed) against a live DITA-OT 4.4 run:

- **`<mapref>` needs no extra code at all.** DITA-OT's own preprocessing
  (its `mapref` build step) flattens the referenced submap's
  `<topicref>` elements directly into the resolved base map before this
  plugin's extractor ever runs — `submap.ditamap` never even enters
  `job.xml` as an `input` map, only as an inert intermediate; the
  extractor's existing recursive map walk (finding 11) picks up the
  merged `<topicref>` tree the same way it picks up ordinary nesting.
- **`anchorref` behaves the same way, with one real caveat.** The
  `<anchor>`/`anchorref` mechanism's *intended* effect — splicing the
  submap's content at the exact position of the named anchor, rather
  than wherever the `<mapref>` textually sits — is **not** honored:
  `spliced` (from `submap.ditamap`) lands as a direct child of
  `user-guide`, not nested under `intro`'s `<topicref>` where the
  `<anchor>` actually is. Confirmed by placing the anchor two different
  ways (at map level, and nested inside a topicref) — both times the
  submap's content merged in at the `<mapref>`'s own location, not the
  anchor's. This doesn't affect this project's graph (`contains` edges
  don't encode sibling order, only which container a topic belongs to,
  and that part is correct here), but it's a real, verified gap from
  DITA's own documented `anchorref` semantics worth stating plainly
  rather than assuming full spec compliance.

`<navref>` is **not** supported, and unlike the two above, would need
real new work: tested the same way, `<navref mapref="nav.ditamap"/>`
survives completely unresolved in DITA-OT's own resolved map output for
this transtype, and the referenced map/topics (`nav.ditamap`/`topics/
nav-topic.dita`) never enter `job.xml` at all — DITA-OT only resolves
`<navref>` inside output-specific transforms (webhelp/eclipsehelp TOC
generation), not generically during preprocessing the way `mapref` is.
Supporting it here would mean this plugin parsing and merging
referenced navigation maps itself, entirely outside DITA-OT's own
pipeline — losing keyref/conref resolution and DITAVAL filtering for
that content specifically — a materially larger, riskier undertaking
than the `mapref`/`anchorref` support confirmed above, and not
attempted (finding 16).

**What changed (finding 16): this gap is no longer silent.**
`DitaModelExtractor.walkMapChildren` detects a `<navref>` element
directly and logs `DITA2GRAPH060W` naming its `mapref`/`href` target,
confirmed to show up in a real `dita` build log:

```
Warning: DITA2GRAPH060W: <navref mapref="nav.ditamap"> in
user-guide.ditamap is not resolved by DITA-OT for this transtype and
is not independently parsed by this plugin; its content is excluded
from the graph
```

`nav-topic` never appears in `graph.json`, `okf/`, or `rag/` — its
absence is the point (same as `Cancel`'s missing `applies-to` edge in
`sample-docs-relations/`), now paired with a warning explaining why,
not a silent omission. Caught and fixed a real encoding bug writing
this message: an earlier draft cited "§3.3" directly in the warning
text, which Ant's console logging mangled into "?3.3" in the actual
build log (confirmed directly) — plain ASCII only in any message
routed through `warn.accept`, matching every other message in this
class.

`graph.json`'s edges for this fixture:

```
user-guide --contains--> intro
user-guide --contains--> after
user-guide --contains--> spliced
```

Exercised by `gradle-build/build.gradle.kts`'s `buildKnowledgeGraphMapref`
task, gated in CI (`.github/workflows/integration.yml`).
