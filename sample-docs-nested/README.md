# sample-docs-nested

A fixture for the nested-map-structure guarantee in
`docs/plugin-specification.md` §3.3/§12 Phase 1 status: a `contains`
edge follows the map's *real* hierarchy, not just its top level, and
DITA-OT's own auto-generated navigation links never get mistaken for
authored cross-references.

`user-guide.ditamap` deliberately exercises all three ways DITA nests
map content:

- `topics/chapter.dita`'s `<topicref>` has a nested child `<topicref>`
  for `topics/section.dita` — `section` should belong to `chapter`
  (`chapter --contains--> section`), not to the map directly.
- `<topichead navtitle="Group">` wraps a `<topicref>` for
  `topics/grouped.dita` — `topichead` has no topic of its own, so the
  `contains` edge skips through it to the map.
- `<topicgroup>` wraps a `<topicref>` for `topics/another.dita` — same
  skip-through behavior as `topichead`.

Confirmed directly (not assumed) against a live DITA-OT 4.4 run that a
genuinely nested map like this one makes DITA-OT auto-generate a
`<related-links>` block of parent/child navigation `<link>` elements in
`chapter.dita`'s and `section.dita`'s *resolved* output — content
`sample-docs/`'s flat map never triggers, since there's no parent/child
map hierarchy for DITA-OT to generate navigation for. Without excluding
`<related-links>` from cross-reference extraction, this pipeline
produced a spurious `chapter --references--> section` /
`section --references--> chapter` pair duplicating (and mislabeling)
the containment relationship as author-declared content, not
auto-generated navigation. `graph.json`'s edges below are what
correctly produced output looks like once that exclusion is applied:

```
user-guide --contains--> chapter
user-guide --contains--> grouped
user-guide --contains--> another
chapter    --contains--> section
```

No `references`/`requires` edges at all — this fixture has no authored
cross-references, only the map hierarchy itself.

Exercised by `gradle-build/build.gradle.kts`'s `buildKnowledgeGraphNested`
task, gated in CI (`.github/workflows/integration.yml`).

Also doubles as the fixture for `args.dita2graph.depth` (§2.3): with
`--args.dita2graph.depth=1`, the nested `chapter --contains--> section`
edge is correctly omitted (level 2, one level past the limit) while
`user-guide --contains--> {chapter, grouped, another}` (level 1) still
appears, and `section` is still extracted as its own node — only its
incoming `contains` edge is missing, the same graceful degradation as
an unresolved topicref target. Confirmed directly against a live
DITA-OT 4.4 run and gated in CI alongside the unlimited-depth check
above.
