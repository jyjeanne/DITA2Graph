# sample-docs-relations

A fixture for the two relation-inference heuristics implemented in
`core/dita2graph-core/src/relations.rs` (`docs/plugin-specification.md`
§3.3, `docs/dev/phase-0-findings.md` finding 15) — `applies-to` and
`generated-from` — checked in after verifying both against a live
DITA-OT 4.4, not assumed from the spec text alone.

## `applies-to`

`topics/save-task.dita` (a task) uses two `<uicontrol>` terms inside its
`<cmd>` elements: `Save` and `Cancel`.

- `Save` also appears in exactly one Reference topic's body
  (`topics/ui-reference.dita`) — **unambiguous**, so `save-task` gets a
  real `applies-to` edge to `ui-reference`.
- `Cancel` appears in **two** Reference topics'
  bodies (`topics/ui-reference.dita` *and*
  `topics/other-ui-reference.dita`) — ambiguous, so per §2.5's
  `DITA2GRAPH010W` ("the lower-confidence edge is dropped, not guessed")
  **no edge is added for it at all**, and a warning is logged instead of
  picking one candidate arbitrarily.

## `generated-from`

`topics/reuser.dita` pulls a paragraph in from `topics/shared-content.dita`
via `<p conref="shared-content.dita#shared-content/warning-note"/>`.
DITA-OT's own resolved output tags that paragraph with an `xtrf`
attribute pointing at `shared-content.dita`, not `reuser.dita` — the
Java extractor (`DitaModelExtractor`) detects this xtrf mismatch and
adds a `generated-from` edge directly (deterministically, not inference
-- see finding 15 for why a plain `keyref` variable substitution does
*not* trigger this, confirmed as a negative case separately).

## `graph.json`

```
save-task --applies-to--> ui-reference
reuser    --generated-from--> shared-content
```

No edge at all involving `other-ui-reference` for the `Cancel` term —
its absence is the point, not an oversight.

Exercised by `gradle-build/build.gradle.kts`'s `buildKnowledgeGraphRelations`
task, gated in CI (`.github/workflows/integration.yml`).
