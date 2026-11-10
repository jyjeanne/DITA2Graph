# Semantic model: RDF, OWL 2 and SHACL

DITA2Graph can write its knowledge graph as RDF, next to the OKF bundle:
the same nodes and relations, described by a published vocabulary (`d2g`,
an OWL 2 RL ontology) and checked by SHACL shapes. The export is opt-in;
the OKF bundle, `graph.json`, `graph.db`, `rag/` and the MCP server are
unchanged by it.

What it is for:

- **Loading the graph anywhere RDF goes**: a triple store, a SPARQL
  endpoint, a knowledge-graph platform, a notebook with rdflib. Every
  relation has a defined meaning, so a graph from one build can be merged
  with others (several publications, another product's graph) without
  guessing what an edge means.
- **Explicit validity rules**: the shapes state what a correct graph is
  (no loop in the table of contents, a prerequisite comes from a task, a
  reuse names fragments of its own source...). Any SHACL engine checks
  them, and CI does on every change.
- **Queries the MCP tools don't anticipate**: SPARQL over the whole graph,
  for audits and one-off questions.

## Producing it

```bash
dita --input user-guide.ditamap --format dita2graph \
  --args.dita2graph.rdf=true \
  --args.dita2graph.rdf.base=https://docs.example.com/user-guide/
```

or, from a normalized model, `dita2graph-core build --input model.json
--output out/ --emit-rdf true --rdf-base https://docs.example.com/user-guide/`.

| Parameter | Default | Effect |
|---|---|---|
| `args.dita2graph.rdf` (`--emit-rdf`) | `false` | Write `rdf/`. A later build without it removes the export's files, so `rdf/` never lags behind `okf/`. |
| `args.dita2graph.rdf.base` (`--rdf-base`) | none | Base IRI of the data. Must be an absolute IRI; a `/` is appended when it ends with neither `/` nor `#`. An invalid base fails the build before anything is written. Without a base, the IRIs use `https://dita2graph.invalid/<map id>/` (`.invalid` is reserved, RFC 2606) and `DITA2GRAPH083W` is logged: fine for local use, not for publishing. |

`SOURCE_DATE_EPOCH` (seconds since the Unix epoch), when set, fixes the
build time recorded in the export and the rest of the bundle: the same
sources then give byte-identical files. The export is sorted, so it does
not depend on the order of the model either.

Files written:

```
rdf/
 ├── graph.ttl      # the data: nodes, relations, source files, the build
 ├── ontology.ttl   # the d2g vocabulary (copy of core/dita2graph-core/ontology/d2g.ttl)
 └── shapes.ttl     # the SHACL shapes (copy of core/dita2graph-core/ontology/d2g-shapes.ttl)
```

`rdf/` goes through the same secret scan as `okf/` and `rag/`
(`DITA2GRAPH050E`).

## Vocabulary

Namespace `https://w3id.org/dita2graph/ns#` (prefix `d2g`), ontology IRI
`https://w3id.org/dita2graph/ns`, version `0.1.0`. It reuses Dublin Core
(`dcterms:`), PROV-O (`prov:`) and SKOS (`skos:`) where they fit.

The ontology stays within the **OWL 2 RL** profile (checked in CI with
ROBOT), so a rule engine can apply it without a full OWL reasoner. It
describes; it does not validate: domains and ranges are broad, and the
rules a graph must follow live in the shapes, checked in a closed world.

### Classes

| Class | Meaning | From |
|---|---|---|
| `d2g:Node` | Anything shown in the publication's outline | — |
| `d2g:Map` ⊑ Node | The input map or bookmap | the map |
| `d2g:Topic` ⊑ Node | A topic, generic when none of the types below | `topic`, unknown types |
| `d2g:Concept`, `d2g:Task`, `d2g:Reference` ⊑ Topic | The DITA topic types | `concept`, `task`, `reference` |
| `d2g:GlossEntry` ⊑ Topic, `skos:Concept` | A glossary entry: a defined term | `glossentry` |
| `d2g:Troubleshooting` ⊑ Topic | Reserved: the extractor exports troubleshooting topics as `d2g:Topic` today | — |
| `d2g:TopicHead` ⊑ Node | A titled outline entry with no topic: `topichead`, a navtitle-only chapter | heading nodes |
| `d2g:Reuse` | One reuse: who pulls which fragments from which source | `generated-from` |
| `d2g:Fragment` | A reused element of a topic, by `@id` (or `element@line` without one) | `generated-from` fragments |
| `d2g:SourceFile` | A source file, by relative path | `sourceFile` |
| `d2g:Key`, `d2g:ExternalResource` | Reserved for keys and external links (not exported yet) | — |

The topic types are pairwise disjoint, and so are `Map`, `Topic` and
`TopicHead`.

### Relations

| Property | From → to | Meaning | Exported today |
|---|---|---|---|
| `d2g:contains` / `d2g:containedIn` | Node → Node | Direct child in the published table of contents; navigation, not dependency. Asymmetric, irreflexive. | Yes, both directions |
| `d2g:references` ⊑ `dcterms:references` | Topic → Node | A link for the reader to follow (`xref`, `link`, relationship tables) | Yes |
| `d2g:requires` ⊑ `references` / `d2g:prerequisiteFor` | Task → Topic | A real prerequisite: a link in the task's `prereq`, or a link marked `importance="required"` | Reserved, see below |
| `d2g:relatedTo` | Topic ↔ Topic | Same product (inferred hint). Symmetric. | Yes |
| `d2g:appliesTo` | Task → Reference | The task operates a UI control the reference describes (inferred) | Yes |
| `d2g:generatedFrom` ⊑ `prov:wasDerivedFrom` | Topic → Topic | Part of the subject's content comes from the object (`conref`, `conkeyref`) | Yes |
| `d2g:qualifiedReuse`, `d2g:reuseSource`, `d2g:reusedFragment`, `d2g:fragmentOf` | Topic → Reuse → Topic, Fragment | Which fragments a `generatedFrom` reuses | Yes |
| `d2g:copyOf` ⊑ `generatedFrom` | Topic → Topic | A `copy-to` copy of the object | Reserved |
| `d2g:definesKey` / `d2g:definedIn`, `d2g:keyTarget`, `d2g:usesKey` | Map, Topic → Key → target | Key definitions and uses | Reserved |
| `d2g:linksTo` | Topic → ExternalResource | A link outside the corpus | Reserved |
| `d2g:includesCode` | Topic → SourceFile | A `coderef` sample file | Reserved |
| `d2g:sourceFile` | Node → SourceFile | The node's file (a heading's file is its map) | Yes |

Each relation carries a `d2g:origin` annotation: `declared` (written by
the author), `derived` (read from DITA-OT's resolved output) or
`inferred` (a heuristic). Weigh `inferred` edges accordingly.

**`requires` today.** The extractor's `requires` currently means "a link
made through a key", not a prerequisite. Until it reads `prereq` and
`importance="required"`, the export writes those links as
`d2g:references`, so no `d2g:requires` triple claims a prerequisite the
author never stated. The OKF bundle and the MCP tools keep their
`requires` edges unchanged.

### Attributes

| Property | On | Value |
|---|---|---|
| `dcterms:identifier` | Node, Fragment | The node id used everywhere else (OKF file name, MCP tools) |
| `dcterms:title`, `dcterms:description` | Node | Title, `shortdesc` |
| `d2g:audience`, `d2g:product` | Topic | One triple per value |
| `d2g:processingRole` | Topic | `"resource-only"` when every topicref naming the topic is resource-only |
| `d2g:ditaClass` | Topic | The root element's `@class`, which keeps a specialization the type folds away |
| `d2g:okfConcept` | Node | The node's OKF concept file, e.g. `okf/topics/<id>.md` |
| `d2g:path` | SourceFile | Path relative to the publication, with `/` |
| `prov:wasGeneratedBy` | Node | The build, a `prov:Activity` with `prov:endedAtTime` and `d2g:generator` |

### IRIs

All data IRIs sit under the base `<b>`; each id is percent-encoded as one
path segment (RFC 3986 unreserved characters kept), each source path
segment by segment.

| Resource | IRI |
|---|---|
| The map | `<b>map/<map id>` |
| A topic or heading | `<b>node/<id>` |
| A fragment | `<b>node/<source id>/fragment/<fragment>` |
| A reuse | `<b>node/<reuser id>/reuse/<source id>` |
| A source file | `<b>file/<relative path>` |
| The build | `<b>build/<timestamp, ':' replaced by '-'>` |
| A key (reserved) | `<b>key/<scope>/<name>` |

A node keeps its IRI across builds as long as its id does not change.

### What the export states explicitly

Nothing is inferred at build time, and SHACL validation runs in a closed
world, so the export writes what queries and shapes need instead of
leaving it to a reasoner: every node's parent classes (`a d2g:Task ,
d2g:Topic , d2g:Node`), both directions of `contains`, and the `Reuse` and
`Fragment` resources qualifying each `generatedFrom`. A plain SPARQL
engine needs no reasoning to query it.

## Validation

`shapes.ttl` holds these shapes (severity Violation unless noted):

| Shape | Rule |
|---|---|
| `NodeShape` | One identifier, one non-empty title, one source file, one OKF concept |
| `SourceFileShape` | One relative path: no leading `/` or `\`, no drive letter, no URL scheme |
| `ContainsShape` | From a map, topic or heading to a topic or heading; no loop |
| `PublishedTopicShape` (Warning) | A topic that is not resource-only has a parent in the table of contents |
| `ReferencesShape` | Target is a node; a topic does not reference itself |
| `RequiresShape` | From a task to a topic; no loop of prerequisites |
| `AppliesToShape` | From a task to a reference |
| `GeneratedFromShape` | Between topics, always qualified by a reuse |
| `ReuseShape`, `FragmentShape` | A reuse has one source and fragments of that source; a fragment has one topic and an identifier |
| `KeyShape`, `UsedKeyShape` | A key has a name, one defining map and one target; a used key is defined |
| `RelatedToShape` (Warning) | `relatedTo` is stated both ways |
| `ExternalResourceShape` (Warning) | An external resource is an `http(s)` URL |

Several rules use SHACL-SPARQL (`sh:sparql`): an engine limited to SHACL
Core skips them silently. pySHACL runs them all:

```bash
pip install pyshacl==0.30.1
pyshacl -s out/rdf/shapes.ttl -e out/rdf/ontology.ttl -i none -w -f human out/rdf/graph.ttl
```

`-w` lets warnings through (exit code 0) and still prints them; without
it, a warning fails the check.

The build does not run the shapes itself yet. CI
(`.github/workflows/integration.yml`) does, on every pull request:

- the golden export `core/dita2graph-core/tests/data/rdf-model.graph.ttl`
  (written by `tests/rdf_export.rs` from `rdf-model.json`, which uses every
  node kind and relation) conforms without a single warning;
- `tests/data/rdf-invalid.ttl`, which breaks one rule per node, must be
  rejected, with each SPARQL rule reported;
- each fixture under `sample-docs*/` is built with the export and
  validated;
- ROBOT checks that `d2g.ttl` stays within OWL 2 RL.

On the two public corpora of the benchmark (`dita2graph-demo`), the export
validates without a violation: the DITA-OT documentation (225 topics,
4,700 triples) with no warning, the Oxygen XML user guide (46,000
triples) with 50 `PublishedTopicShape` warnings: 46 topics merged into
their parent by `chunk="to-content"`, 3 `copy-to` sources (every topicref
naming them publishes a copy, so the original itself is outside the
table of contents) and 1 duplicate id. Each one is a real case worth a
look, not noise.

## Querying

Any SPARQL 1.1 engine works. Examples, with these prefixes:

```sparql
PREFIX d2g: <https://w3id.org/dita2graph/ns#>
PREFIX dcterms: <http://purl.org/dc/terms/>
```

Which topics reuse fragments of a topic, and which ones (impact of an
edit):

```sparql
SELECT ?reuser ?fragment WHERE {
  ?source dcterms:identifier "warehouse" .
  ?reuser d2g:qualifiedReuse ?reuse .
  ?reuse d2g:reuseSource ?source ; d2g:reusedFragment/dcterms:identifier ?fragment .
}
```

The outline above a topic, from the map down:

```sparql
SELECT ?title WHERE {
  ?topic dcterms:identifier "glossary-anchor" .
  ?ancestor d2g:contains+ ?topic ; dcterms:title ?title .
}
```

Tasks that operate a reference's controls, with their audience:

```sparql
SELECT ?task ?reference ?audience WHERE {
  ?task d2g:appliesTo ?reference .
  OPTIONAL { ?task d2g:audience ?audience }
}
```

## Limits and next steps

- Reserved terms (`requires` in its new sense, keys, external links,
  `coderef`, `copyOf`, `Troubleshooting`) are declared and validated but
  not exported yet: the extractor does not collect them. They come with
  the extractor changes of the next phase. With `copyOf` exported,
  `PublishedTopicShape` can also stop warning about the source of a
  `copy-to` copy that is published only through its copies.
- The shapes run in CI, not in the build. A build-time check, with its
  own diagnostics (`DITA2GRAPH080E` for a violation, `081W` and `082I` for
  warnings and notes), is planned.
- The vocabulary is versioned (`owl:versionIRI`): a term's meaning does
  not change within a version; a change gets a new version.
