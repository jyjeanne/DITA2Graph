//! Tool definitions and dispatch (§5.2, §5.5): `tools/list`'s result and
//! the implementation behind `tools/call`, adapted from the `okf-mcp`
//! pattern in `docs/plugin-specification.md` §5.5 — swap `okf-query`'s
//! generic graph/search calls for the DITA-relation-aware ones here.

use crate::bundle::{BundleCache, BundleReader};
use crate::live::{self, LiveValidationConfig};
use anyhow::{Result, anyhow};
use dita2graph_core::graph_view::MISSING_TYPE;
use serde_json::{Value, json};
use std::path::Path;

/// The tool list; `query_graph` only when the server runs with `--sparql`.
pub fn list(sparql: bool) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "search_topics",
            "description": "Free-text search over the titles and ids of topics and maps, and of the keys (id key:<name>), external URLs (id = the URL) and sample files (id file:<path>) they declare. Titles and ids that contain the query as a phrase come first. For a query of several words, those that have every word (each at the start of a word, in any order, stop words such as 'how', 'to', 'the' left out) follow under 'Title or id with every word of': 'rename id' finds 'Search and Rename Actions for IDs'. Then the authors' keywords and index entries: a topic whose title does not match but whose keyword or index entry has a word starting with the query, or every word of it, is listed under 'Indexed under or tagged with', with the entries that matched. A query that looks like a file (topics/x.dita) also lists, under 'Source file matching', the topics whose source file it names, and the copies the map publishes of one. Use this first to find an id before calling the other tools; every tool that takes a topicId also takes a topic's source file path or a key name, and says which topic that resolved to. Every tool marks a topic the map uses only as a resource (a reuse warehouse, a key target) with [resource-only: not a published page]: its content may appear elsewhere, but it is not a page of the publication. A file a key names or a topic pushes content into that DITA-OT could not load (id file:<path>) is marked [not in this build]: often a page the documentation's own build generates before publishing.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Search text" } },
                "required": ["query"],
            },
        }),
        json!({
            "name": "read_topic",
            "description": "The full text of one topic, as a reader of the published page sees it: summary and body, with every fragment it reuses by conref/conkeyref kept in place between [reused from <file>#<fragment>] and [end reused] markers, then the list of reused fragments by source topic. Text between [draft-comment] and [end draft-comment] (or [required-cleanup] and [end required-cleanup]) is a note for the authors, which the output leaves out unless the build sets args.draft: readers do not see it by default. Text between [pushed after <file>#<element>] (or pushed before, pushed to replace) and [end pushed] is pushed into that other page with conref push: readers see it there, even when this topic is resource-only; the pushed passages are listed by page after the text. A file:<path> id marked [not in this build] is a page DITA-OT could not load (one the documentation's own build generates, or a missing one): read_topic on it says which keys name it and what is pushed into it. Use this whenever the answer is in what a topic says -- the other tools only return short excerpts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topicId": { "type": "string", "description": "Concept id (find it with search_topics or search_content)" },
                    "maxChars": { "type": "integer", "description": "Truncate the text after this many characters (default 20000)" },
                },
                "required": ["topicId"],
            },
        }),
        json!({
            "name": "find_related_topics",
            "description": "List what the given id is related to, optionally filtered to one relation: contains/references/requires/related-to/applies-to/generated-from/copy-of between topics (§4.3), uses-key/links-to/includes-code from a topic to a key, URL or sample file, defines-key from the map, key-target from a key. A generated-from relation also names the reused fragments (element ids in the source topic). For a key, URL or sample file id, also lists what points at it: who uses the key, who links to the URL, which key names it, and the file that defines the key.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topicId": { "type": "string", "description": "Concept id (find it with search_topics)" },
                    "relation": { "type": "string", "description": "Optional relation to filter to" },
                },
                "required": ["topicId"],
            },
        }),
        json!({
            "name": "explain_task",
            "description": "Title, description, a body excerpt, and requires/contains/applies-to relations for a topic id. For the topic's full text, use read_topic.",
            "inputSchema": {
                "type": "object",
                "properties": { "topicId": { "type": "string" } },
                "required": ["topicId"],
            },
        }),
        json!({
            "name": "trace_dependencies",
            "description": "Follow `requires` edges (prerequisites: a task's prereq links, or its links marked importance=\"required\") from a topic id up to a given depth.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topicId": { "type": "string" },
                    "depth": { "type": "integer", "description": "Max traversal depth (default 3)" },
                },
                "required": ["topicId"],
            },
        }),
        json!({
            "name": "search_content",
            "description": "Full-text search over rag/chunks.jsonl's topic body/summary text (§13.1) -- unlike search_topics (title/id only), this searches actual content, and each hit includes a text excerpt. Ranked by keyword frequency, plus cosine similarity against rag/embeddings.jsonl when this server was started with an embedding model configured -- a strong semantic match can surface even with no shared words, not just literal term overlap. Returns at most the top 15 matches by relevance; pass topicId (optionally with relation/depth) to narrow the search to topics reachable from that id via the graph first: the hybrid pattern in §13.1 -- cheap, deterministic graph narrowing, then content search only within that smaller set, instead of the whole bundle. Excerpts are 200 characters: call read_topic on a hit to read it in full. To list every topic that contains a given word, command or name, with no cap, use find_text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "topicId": { "type": "string", "description": "Optional: narrow the search to topics reachable from this id" },
                    "relation": { "type": "string", "description": "Optional: restrict narrowing to one relation (default: all)" },
                    "depth": { "type": "integer", "description": "Max narrowing depth from topicId (default 3)" },
                },
                "required": ["query"],
            },
        }),
        json!({
            "name": "find_text",
            "description": "Every topic whose text contains the given text, literally (ASCII letters case-insensitive), with no cap: titles, summaries and bodies, reused passages included. Use it when a question asks for every page that shows a command, a name, a property or a phrase, or whether something is spelled the same everywhere -- search_content only returns the 15 best matches. Gives the number of occurrences and of topics, published pages apart from resource-only topics, up to 3 occurrences per topic in context, the reused passage (<file>#<fragment>) an occurrence comes from -- edit it once and every topic reusing it changes -- and, for a single word, the near spellings that appear in the documentation (one or two letters apart), which the count does not include. It also names the keys that define the text -- whose link text holds it, or whose target URL does -- with the map that defines each: a page showing it through a key changes with the key's definition, not with its own text. An occurrence inside a draft-comment or required-cleanup element is marked as a note for the authors, which the output leaves out unless the build sets args.draft; search draft-comment to list those notes. An occurrence in a passage the topic pushes into another page with conref push says so, with that page: readers see it there, even when the topic holding it is resource-only. The list is complete for this exact text only: other wordings of the same thing need their own search. Paginated by topic (50 by default; pass offset for the next page).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to find, as written (a word, a command, a name or a phrase)" },
                    "limit": { "type": "integer", "description": "Topics per page (default 50; 0 = counts only)" },
                    "offset": { "type": "integer", "description": "Index of the first topic to return (default 0)" },
                },
                "required": ["text"],
            },
        }),
        json!({
            "name": "analyze_impact",
            "description": "Find the concepts that changing a topic id may affect -- or a key (key:<name>), an external URL or a sample file (file:<path>): everything that points at it directly (containing maps, referrers and prerequisites, topics using a key, linking to a URL directly or through a key, or including a file), and the topics whose content reuses it (a topic reusing a passage that holds some of it reuses it too, directly; a page it pushes content into with conref push reuses it too, even one marked [not in this build]). A concept that only links to, contains or uses a key to one of those, or reuses another part of one, keeps its own content: the first line counts these apart, and cascade=true lists them after the others. Closest first, with a text excerpt under each affected concept. The first line gives the total count, by relation and by level, so 'how many' needs no paging. Results are paginated (40 entries by default: pass offset for the next page). To find the topics that reuse one fragment of a conref source, pass fragment=<fragment id> with depth=1.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topicId": { "type": "string" },
                    "depth": { "type": "integer", "description": "Max traversal depth (default 5)" },
                    "relation": { "type": "string", "description": "Optional: follow only this relation (e.g. generated-from, references, contains)" },
                    "fragment": { "type": "string", "description": "Optional: keep only the direct dependents that reuse this fragment of topicId (its element id, as shown by find_related_topics)" },
                    "cascade": { "type": "boolean", "description": "Also list the concepts only connected to an affected one (a link, a map, a key, or reuse of its other parts), up to depth (default false: they are only counted)" },
                    "limit": { "type": "integer", "description": "Entries per page (default 40; 0 = counts only)" },
                    "offset": { "type": "integer", "description": "Index of the first entry to return (default 0)" },
                    "excerpts": { "type": "boolean", "description": "Include a text excerpt under each entry (default true)" },
                },
                "required": ["topicId"],
            },
        }),
        json!({
            "name": "generate_summary",
            "description": "Title and description for a topic or map id.",
            "inputSchema": {
                "type": "object",
                "properties": { "topicId": { "type": "string" } },
                "required": ["topicId"],
            },
        }),
        json!({
            "name": "validate_bundle",
            "description": "Re-run okf-validator conformance checks against the bundle this server is bound to (§2.5, §6.4, §10).",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "validate_live",
            "description": "Runs jyjeanne/ditacraft's live LSP validation pipeline (DTD/RNG, 43 Schematron-equivalent rules, cross-reference, circular-reference, subject-scheme profiling -- the same checks DitaCraft runs as-you-type in VS Code) against a topic id's *current* on-disk DITA source file, not the last-built bundle. Complements validate_bundle (which only re-checks the build-time OKF/secret-leak gates) with finer-grained, editor-grade diagnostics. Requires --source-root/DITA2GRAPH_SOURCE_ROOT to be configured, and a `node` binary on PATH.",
            "inputSchema": {
                "type": "object",
                "properties": { "topicId": { "type": "string" } },
                "required": ["topicId"],
            },
        }),
    ];
    if sparql {
        tools.push(query_graph_definition());
    }
    tools
}

/// `query_graph`'s definition: the description carries the vocabulary an
/// agent needs to write a query without reading the ontology first.
fn query_graph_definition() -> Value {
    json!({
        "name": "query_graph",
        "description": concat!(
            "Read-only SPARQL 1.1 query (SELECT, ASK, CONSTRUCT, DESCRIBE) over the bundle's RDF graph: ",
            "every node, relation, key, external link and sample file, with the d2g vocabulary ",
            "(docs/semantic-model.md). For structural questions the other tools need several calls for: ",
            "paths (d2g:contains+, d2g:requires+), counts (COUNT, GROUP BY), joins across keys, links and reuse. ",
            "Not for what a topic says: use read_topic. ",
            "Predeclared prefixes: d2g:, dcterms:, prov:, skos:, rdf:, rdfs:, owl:, xsd:, and for this bundle's ",
            "data node: (topics and headings, node:<id>), map: (map:<id>) and key: (root-scope keys, key:<name>); ",
            "find a node by id with ?t dcterms:identifier \"<id>\". ",
            "Classes: d2g:Node, d2g:Map, d2g:Topic and its types d2g:Concept, d2g:Task, d2g:Reference, ",
            "d2g:GlossEntry, d2g:Troubleshooting, d2g:TopicHead, d2g:Key, d2g:ExternalResource (the URL itself), ",
            "d2g:SourceFile, d2g:Reuse, d2g:Fragment. ",
            "Relations: d2g:contains/d2g:containedIn (table of contents), d2g:references, d2g:requires ",
            "(prerequisites, also references), d2g:generatedFrom with d2g:qualifiedReuse -> d2g:Reuse ",
            "(d2g:reuseSource, d2g:reusedFragment -> d2g:Fragment with dcterms:identifier), d2g:copyOf, ",
            "d2g:relatedTo, d2g:appliesTo, d2g:definesKey/d2g:definedIn (map <-> key), d2g:keyTarget ",
            "(topic, URL or file), d2g:usesKey, d2g:linksTo (URL), d2g:includesCode (file), d2g:sourceFile ",
            "(also the map file that defines a key). Attributes: dcterms:identifier, dcterms:title, ",
            "dcterms:description, d2g:processingRole (\"resource-only\": not a published page), d2g:audience, ",
            "d2g:product, d2g:ditaClass, d2g:keyName, d2g:keyText, d2g:path, d2g:okfConcept. ",
            "Results come a page at a time (limit, default 50, at most 500; offset), with the total first; ",
            "a query is stopped after 10 seconds."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The SPARQL query" },
                "limit": { "type": "integer", "description": "Results per page (default 50, at most 500)" },
                "offset": { "type": "integer", "description": "Index of the first result to return (default 0)" },
            },
            "required": ["query"],
        },
    })
}

fn query_graph(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let query = arg_str(arguments, "query")?;
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map_or(crate::sparql::DEFAULT_LIMIT, |v| v as usize);
    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    bundle
        .sparql_graph()?
        .query(query, limit, offset, crate::sparql::TIMEOUT)
}

/// Dispatches one `tools/call`. `validate_bundle` and `validate_live`
/// are deliberately special-cased ahead of `cache.get()`'s single
/// `match`: `validate_bundle` is meant to check the bundle's live
/// on-disk state (`§2.5`/`§6.4`/`§10`), not go through the cached
/// `BundleReader` at all -- and unlike every other tool here, it
/// doesn't need `graph.json` to exist to do its job, so it shouldn't
/// fail just because a `dita2graph-core build` hasn't produced one yet.
/// `validate_live` needs both the cached `BundleReader` (to resolve a
/// topic id to its source file) *and* `cache`'s `live_config()` --
/// cloned out before `cache.get()`'s `&mut self` borrow, since the two
/// can't be held at once. `search_content` needs the same treatment for
/// `cache.embedder()`.
pub fn call(name: &str, arguments: &Value, cache: &mut BundleCache) -> Result<String> {
    if name == "validate_bundle" {
        return validate_bundle(cache.root());
    }
    if name == "validate_live" {
        let live_config = cache.live_config().clone();
        let bundle = cache.get()?;
        return with_resolved_topic(bundle, arguments, |arguments| {
            validate_live(bundle, arguments, &live_config)
        });
    }
    if name == "query_graph" && !cache.sparql_enabled() {
        return Err(anyhow!(
            "query_graph is not enabled on this server (start dita2graph-mcp with --sparql)"
        ));
    }
    let embedder = cache.embedder();
    let bundle = cache.get()?;
    with_resolved_topic(bundle, arguments, |arguments| match name {
        "query_graph" => query_graph(bundle, arguments),
        "search_topics" => search_topics(bundle, arguments),
        "search_content" => search_content(bundle, arguments, embedder.as_deref()),
        "find_text" => find_text(bundle, arguments),
        "read_topic" => read_topic(bundle, arguments),
        "find_related_topics" => find_related_topics(bundle, arguments),
        "explain_task" => explain_task(bundle, arguments),
        "trace_dependencies" => trace_dependencies(bundle, arguments),
        "analyze_impact" => analyze_impact(bundle, arguments),
        "generate_summary" => generate_summary(bundle, arguments),
        other => Err(anyhow!("unknown tool: {other}")),
    })
}

/// Runs a tool with its `topicId` resolved when it is a topic's source
/// file path or a key name rather than an id ([`resolve_topic_id`]), the
/// result then opening with what it was resolved to.
fn with_resolved_topic(
    bundle: &BundleReader,
    arguments: &Value,
    run: impl FnOnce(&Value) -> Result<String>,
) -> Result<String> {
    let resolved = match arguments.get("topicId").and_then(|v| v.as_str()) {
        Some(raw) => resolve_topic_id(bundle, raw)?,
        None => None,
    };
    let Some(resolved) = resolved else {
        return run(arguments);
    };
    let mut arguments = arguments.clone();
    arguments["topicId"] = json!(resolved.id);
    run(&arguments).map(|text| format!("{}\n\n{text}", resolved.note))
}

/// A `topicId` resolved from a path or a key, and the line saying so.
struct Resolved {
    id: String,
    note: String,
}

/// What a `topicId` that is not an id names: a key (`using-dita-command`
/// or `key:using-dita-command`) naming one topic, or a topic's source file
/// (`topics/using-dita-command.dita`, `using-dita-command.dita`, or a full
/// path ending with it), and then which copies the map publishes of it.
/// `None` when it names neither, so the tool reports the unknown id as
/// before; an error when it names several topics. Found by the relevance
/// checks (dita2graph-demo/bench, R4): asked which pages change when
/// `topics/using-dita-command.dita` is edited, Haiku never got from the
/// path to its topic, `using-command-line-tool`, a resource-only topic
/// the map publishes as two copies, and analysed a neighbouring one.
fn resolve_topic_id(bundle: &BundleReader, raw: &str) -> Result<Option<Resolved>> {
    let raw = raw.trim();
    if raw.is_empty() || bundle.node(raw).is_some() {
        return Ok(None);
    }
    let name = raw.strip_prefix("key:").unwrap_or(raw);
    let key_id = format!("key:{name}");
    if let Some(key) = bundle.node(&key_id) {
        let targets: Vec<&str> = bundle
            .edges_from(&key_id, Some("key-target"))
            .into_iter()
            .map(|edge| edge.to.as_str())
            .filter(|target| bundle.node(target).is_some_and(|n| !n.is_declared()))
            .collect();
        let defined = key
            .source_file
            .as_deref()
            .map(|file| format!(", defined in {file},"))
            .unwrap_or_default();
        if let [target] = targets.as_slice() {
            return Ok(Some(Resolved {
                id: target.to_string(),
                note: format!(
                    "`{raw}` is the key `{name}`{defined} which names {}.{}",
                    describe_node(bundle, target),
                    copies_note(bundle, target)
                ),
            }));
        }
        // A key naming a topic file the build does not have.
        if let Some(edge) = bundle
            .edges_from(&key_id, Some("key-target"))
            .into_iter()
            .find(|edge| {
                bundle
                    .node(&edge.to)
                    .is_some_and(|n| n.type_ == MISSING_TYPE)
            })
        {
            return Ok(Some(Resolved {
                id: edge.to.clone(),
                note: format!(
                    "`{raw}` is the key `{name}`{defined} which names {}{MISSING_NOTE}.",
                    edge.to
                ),
            }));
        }
    }
    let files = bundle.source_files();
    let own: Vec<&crate::bundle::SourceFile> = files
        .iter()
        .filter(|f| !f.copied_from && names_file(raw, &f.path))
        .collect();
    match own.as_slice() {
        [file] => {
            return Ok(Some(Resolved {
                id: file.id.clone(),
                note: format!(
                    "`{raw}` is the source file {} of {}.{}",
                    file.path,
                    describe_node(bundle, &file.id),
                    copies_note(bundle, &file.id)
                ),
            }));
        }
        [] => {}
        several => {
            let listed: Vec<String> = several
                .iter()
                .map(|f| format!("{} ({})", describe_node(bundle, &f.id), f.path))
                .collect();
            return Err(anyhow!(
                "`{raw}` matches the source files of several topics: {}. Pass one of their ids.",
                listed.join("; ")
            ));
        }
    }
    let copies: Vec<&str> = files
        .iter()
        .filter(|f| f.copied_from && names_file(raw, &f.path))
        .map(|f| f.id.as_str())
        .collect();
    match copies.as_slice() {
        // A topic file the build does not have, named by a key or a push.
        [] => Ok(bundle
            .all_nodes()
            .find(|n| {
                n.type_ == MISSING_TYPE
                    && n.title.as_deref().is_some_and(|path| names_file(raw, path))
            })
            .map(|n| Resolved {
                id: n.id.clone(),
                note: format!("`{raw}` is {}{MISSING_NOTE}.", n.id),
            })),
        [id] => Ok(Some(Resolved {
            id: id.to_string(),
            note: format!(
                "`{raw}` has no topic of its own: the map publishes it as one copy, {}.",
                describe_node(bundle, id)
            ),
        })),
        several => {
            let listed: Vec<String> = several.iter().map(|id| describe_node(bundle, id)).collect();
            Err(anyhow!(
                "`{raw}` has no topic of its own: the map publishes it as {} copies, {}. Pass \
                 one of their ids.",
                several.len(),
                listed.join("; ")
            ))
        }
    }
}

/// `Title [id]`, with the resource-only note.
fn describe_node(bundle: &BundleReader, id: &str) -> String {
    format!(
        "{} [{id}]{}",
        bundle.title(id).unwrap_or_else(|_| id.to_string()),
        role_note(bundle, id)
    )
}

/// ` The pages built from it are its copies: …` when the map publishes
/// `id` under other names (`copy-of` edges to it), empty otherwise: the
/// copies, not the file's own topic, are what readers see.
fn copies_note(bundle: &BundleReader, id: &str) -> String {
    let mut copies: Vec<&str> = bundle
        .edges_to(id, Some("copy-of"))
        .into_iter()
        .map(|edge| edge.from.as_str())
        .collect();
    copies.sort_unstable();
    copies.dedup();
    match copies.as_slice() {
        [] => String::new(),
        [copy] => format!(
            " The map publishes it as one copy, the page readers see: {}.",
            describe_node(bundle, copy)
        ),
        several => format!(
            " The map publishes it as {} copies, the pages readers see: {}.",
            several.len(),
            several
                .iter()
                .map(|copy| describe_node(bundle, copy))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

/// `path` with `\` read as `/`, no leading `./`, ASCII-lowercased.
fn normalize_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

/// Whether the file path `query` names `path`: the same path, or one that
/// ends with the other at a `/` (`using-dita-command.dita`, or
/// `C:/docs/topics/using-dita-command.dita`, for
/// `topics/using-dita-command.dita`).
fn names_file(query: &str, path: &str) -> bool {
    let (query, path) = (normalize_path(query), normalize_path(path));
    !query.is_empty()
        && (query == path
            || query.ends_with(&format!("/{path}"))
            || path.ends_with(&format!("/{query}")))
}

fn arg_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing required argument `{key}`"))
}

fn search_topics(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let query = arg_str(arguments, "query")?.to_lowercase();
    let words = query_words(&query);
    let mut hits: Vec<String> = Vec::new();
    // A node whose title or id has every word of a multi-word query, in
    // any order, but not the query as one phrase: "rename id" finds
    // "Search and Rename Actions for IDs in DITA", which a live session
    // only reached by trying "rename" alone (bench r-o3, Oxygen guide).
    let mut by_words: Vec<String> = Vec::new();
    // A node found by its keywords or index entries rather than its name,
    // with what matched: the author's own words for what the topic is
    // about, which its title often does not repeat.
    let mut by_metadata: Vec<String> = Vec::new();
    for node in bundle.all_nodes() {
        let title = bundle.title(&node.id).unwrap_or_else(|_| node.id.clone());
        let line = format!(
            "{} ({}) [{}]{}",
            title,
            node.type_,
            node.id,
            role_note(bundle, &node.id)
        );
        let (id, title) = (node.id.to_lowercase(), title.to_lowercase());
        if id.contains(&query) || title.contains(&query) {
            hits.push(line);
        } else if !words.is_empty()
            && words
                .iter()
                .all(|w| starts_a_word(&title, w) || starts_a_word(&id, w))
        {
            by_words.push(line);
        } else if !node.is_declared() {
            let matched = metadata_matches(&bundle.metadata(&node.id), &query, &words);
            if !matched.is_empty() {
                by_metadata.push(format!("{line} -- {matched}"));
            }
        }
    }
    // A query that looks like a file: the topics whose source file it
    // names or is part of, and the copies made of one (relevance check R4:
    // a session searched `topics/using-dita-command.dita` and found nothing).
    let mut by_file: Vec<String> = Vec::new();
    if query.contains(['.', '/', '\\']) {
        let wanted = normalize_path(&query);
        for file in bundle.source_files().iter() {
            if !(normalize_path(&file.path).contains(&wanted) || names_file(&query, &file.path)) {
                continue;
            }
            let type_ = bundle.node(&file.id).map_or("", |n| n.type_.as_str());
            by_file.push(format!(
                "{} ({type_}) [{}]{} -- {} {}",
                bundle.title(&file.id).unwrap_or_else(|_| file.id.clone()),
                file.id,
                role_note(bundle, &file.id),
                if file.copied_from {
                    "a copy of"
                } else {
                    "source file"
                },
                file.path
            ));
        }
    }
    hits.sort();
    by_words.sort();
    by_metadata.sort();
    if hits.is_empty() && by_words.is_empty() && by_metadata.is_empty() && by_file.is_empty() {
        return Ok(format!("no topics matched `{query}`"));
    }
    for (heading, mut group) in [
        (format!("Source file matching `{query}`"), by_file),
        (
            format!("Title or id with every word of `{query}`"),
            by_words,
        ),
        (
            format!("Indexed under or tagged with `{query}`"),
            by_metadata,
        ),
    ] {
        if group.is_empty() {
            continue;
        }
        if !hits.is_empty() {
            hits.push(String::new());
        }
        hits.push(format!("{heading} ({}):", group.len()));
        hits.append(&mut group);
    }
    Ok(hits.join("\n"))
}

/// Words that say nothing about a topic, left out of a multi-word query's
/// words ("how to rename an id" looks for "rename" and "id").
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "how",
    "i", "in", "into", "is", "it", "my", "of", "on", "or", "the", "this", "to", "what", "when",
    "where", "which", "with",
];

/// The words of `query` a node must all have when it does not have the
/// query as one phrase: its words, lowercased, without the stop words
/// (all of them when there is nothing else), each once. Empty for a
/// one-word query, which the phrase already covers. A compound with `-`,
/// `_` or `.` stays one word: `plug-ins` split would look for "ins",
/// which starts "installing".
fn query_words(query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    let tokens: Vec<&str> = query
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .map(|t| t.trim_matches(|c: char| matches!(c, '-' | '_' | '.')))
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() < 2 {
        return Vec::new();
    }
    let mut words: Vec<String> = tokens
        .iter()
        .filter(|t| !STOP_WORDS.contains(t))
        .map(|t| t.to_string())
        .collect();
    if words.is_empty() {
        words = tokens.iter().map(|t| t.to_string()).collect();
    }
    words.sort();
    words.dedup();
    words
}

/// What of a node's keywords and index entries has a word starting with
/// `query` (already lowercased), or, for a multi-word query, every one of
/// its `words` (the terms of one index entry taken together):
/// `keyword "..."` and `index "term > subterm"`, the first three, then how
/// many more; empty when nothing does. A word start, not any substring as
/// for titles: on the Oxygen guide, `git` otherwise finds the "Digital
/// Signature" entries.
fn metadata_matches(m: &dita2graph_core::Metadata, query: &str, words: &[String]) -> String {
    const SHOWN: usize = 3;
    let phrase = |term: &str| starts_a_word(&term.to_lowercase(), query);
    let every_word = |term: &str| {
        let term = term.to_lowercase();
        !words.is_empty() && words.iter().all(|w| starts_a_word(&term, w))
    };
    let keywords = m
        .keywords
        .iter()
        .filter(|k| phrase(k) || every_word(k))
        .map(|k| format!("keyword \"{k}\""));
    let entries = m
        .index_terms
        .iter()
        .filter(|path| path.iter().any(|term| phrase(term)) || every_word(&path.join(" > ")))
        .map(|path| format!("index \"{}\"", path.join(" > ")));
    let mut matched: Vec<String> = keywords.chain(entries).collect();
    matched.dedup();
    let more = matched.len().saturating_sub(SHOWN);
    matched.truncate(SHOWN);
    if more > 0 {
        matched.push(format!("+{more} more"));
    }
    matched.join(", ")
}

/// `query` occurs in `text` at the start of a word: at the start of
/// `text`, or after a character that is not a letter or a digit.
fn starts_a_word(text: &str, query: &str) -> bool {
    !query.is_empty()
        && text.match_indices(query).any(|(i, _)| {
            text[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric())
        })
}

/// A semantic-similarity match this strong counts on its own, even with
/// zero keyword overlap -- a real paraphrase (different words, same
/// meaning) is exactly the gap keyword-frequency ranking can't close
/// (§13.1's "only node-level embeddings... remain"). Below this, a
/// keyword hit is still required: cosine similarity between two
/// unrelated real-model embeddings rarely sits at zero, so treating any
/// positive similarity as a match would flood results with noise.
const SEMANTIC_MATCH_THRESHOLD: f32 = 0.5;
/// How much a full-strength (cosine similarity 1.0) semantic match adds
/// to the combined score, relative to `relevance_score`'s scale (+5 for
/// a title hit, +1 per body occurrence) -- large enough that a strong
/// paraphrase match can outrank a single incidental body mention, not so
/// large that it always drowns out a real, repeated keyword match.
const SEMANTIC_WEIGHT: f64 = 8.0;

/// Content search over `rag/chunks.jsonl` (§13.1), optionally scoped to
/// the topics reachable from `topicId` via a forward graph traversal --
/// the "graph narrows first, content search runs only on what's left"
/// pattern that section describes, made concrete: `search_topics` above
/// only ever matches titles/ids against `okf/`, never a topic's actual
/// prose. Results are ranked by a combined score: keyword-frequency (see
/// `relevance_score`) plus, when `embedder` is configured and
/// `rag/embeddings.jsonl` exists, cosine similarity against the query's
/// own embedding (§13.1's node-level embeddings, the one piece that
/// section left unimplemented) -- not returned in an arbitrary/
/// alphabetical order the way `search_topics` still is. With no embedder
/// configured (the default), this reduces to exactly the keyword-only
/// behavior from before embeddings existed.
fn search_content(
    bundle: &BundleReader,
    arguments: &Value,
    embedder: Option<&dita2graph_core::Embedder>,
) -> Result<String> {
    let query = arg_str(arguments, "query")?.to_lowercase();
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() {
        return Err(anyhow!(
            "`query` must contain at least one non-whitespace term"
        ));
    }
    let scope_topic = arguments.get("topicId").and_then(|v| v.as_str());
    let relation = arguments.get("relation").and_then(|v| v.as_str());
    let depth = arguments.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

    let chunks = bundle.rag_chunks()?;
    if chunks.is_empty() {
        return Ok(
            "no rag/chunks.jsonl found for this bundle -- run `dita2graph-core build` \
             (not just `validate`) to produce one (§13.1)"
                .to_string(),
        );
    }

    // Best-effort: an embedder that fails on this particular query text
    // (or a bundle with no rag/embeddings.jsonl) degrades to keyword-only
    // ranking rather than failing the whole search -- the same
    // graceful-degradation discipline `rag_chunks()`/`embeddings()`
    // themselves already apply to a missing file.
    let query_embedding = embedder.and_then(|e| e.embed(&query).ok());
    let embeddings = if query_embedding.is_some() {
        bundle.embeddings().unwrap_or_default()
    } else {
        Default::default()
    };

    let allowed = scope_topic.map(|id| forward_reachable(bundle, id, relation, depth));

    let mut scored: Vec<(f64, &str, &str, &str, Option<String>)> = Vec::new();
    for chunk in chunks.iter() {
        if let Some(allowed) = &allowed
            && !allowed.contains(&chunk.id)
        {
            continue;
        }
        let title_lower = chunk.title.to_lowercase();
        let text_lower = chunk.text.as_deref().unwrap_or_default().to_lowercase();
        let keyword_score = relevance_score(&terms, &title_lower, &text_lower);

        let semantic_score = query_embedding.as_deref().and_then(|q| {
            embeddings
                .get(&chunk.id)
                .map(|v| dita2graph_core::cosine_similarity(q, v))
        });
        let semantic_match = semantic_score.is_some_and(|s| s >= SEMANTIC_MATCH_THRESHOLD);
        if keyword_score == 0 && !semantic_match {
            continue;
        }
        let combined =
            keyword_score as f64 + semantic_score.unwrap_or(0.0).max(0.0) as f64 * SEMANTIC_WEIGHT;

        scored.push((
            combined,
            chunk.id.as_str(),
            chunk.title.as_str(),
            chunk.topic_type.as_str(),
            // Found live: a real Claude Code session searched for
            // content, got back title/id/score for every hit, and
            // still had no way to see *what actually matched*
            // without a second round trip -- and no other tool
            // fills that gap either (explain_task/generate_summary
            // only ever surface title + shortdesc, never body).
            // This is the one place `search_content` can answer
            // "what does this topic actually say" directly, so it
            // should.
            chunk.text.as_deref().map(|t| excerpt(t, 200)),
        ));
    }
    // Highest score first; tie-break by id so the ordering is
    // deterministic rather than dependent on chunks.jsonl's file order.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));

    if scored.is_empty() {
        let query = terms.join(" ");
        return Ok(match scope_topic {
            Some(id) => format!("no content matched `{query}` within topics reachable from `{id}`"),
            None => format!("no content matched `{query}`"),
        });
    }

    // Capped at the top MAX_RESULTS matches -- found live: on a real,
    // sizeable corpus, an unscoped or broad query matching dozens of
    // topics, each now carrying its own excerpt (above), produced
    // 50+ KB of output, well past what a real MCP client would render
    // inline -- the exact "small, typed results, not raw file dumps"
    // failure mode this whole tool set exists to avoid. The excerpts
    // are what make results actually useful (see above), so the fix is
    // capping result *count*, not dropping the excerpts back out.
    // Narrowing scope with topicId is the documented way to see more of
    // a specific area; a truncation note here tells the caller that
    // option exists rather than silently hiding the rest.
    const MAX_RESULTS: usize = 15;
    let total = scored.len();
    scored.truncate(MAX_RESULTS);

    let mut out = scored
        .into_iter()
        .map(|(score, id, title, topic_type, text_excerpt)| {
            let mut line = format!(
                "{title} ({topic_type}) [{id}]{} (score: {score:.2})",
                role_note(bundle, id)
            );
            if let Some(text_excerpt) = text_excerpt {
                line.push_str(&format!("\n  {text_excerpt}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    if total > MAX_RESULTS {
        out.push_str(&format!(
            "\n\n(showing top {MAX_RESULTS} of {total} matches -- narrow with topicId to see more of a specific area)"
        ));
    }
    Ok(out)
}

/// Topics per page of [`find_text`]. Each shows at most
/// [`FIND_TEXT_SNIPPETS`] occurrences, so a page stays a few kilobytes.
const FIND_TEXT_PAGE: usize = 50;
const FIND_TEXT_SNIPPETS: usize = 3;
/// Bytes of context on each side of an occurrence in [`find_text`].
const FIND_TEXT_CONTEXT: usize = 70;

/// Every topic whose text holds `text`, literally, with no cap -- what
/// `search_content` cannot give: it ranks by keyword and stops at 15. Found
/// by the relevance checks (dita2graph-demo/bench, 26 November 2026): asked
/// whether `plugindirs` is spelled the same everywhere, models searched for
/// it and its likely variants, never met the `pluginsdir` two published
/// pages show, and answered "the same everywhere"; asked for every page that
/// shows `dita install`, they read the first 15 hits and missed pages. So
/// the answer says it is complete, which reused passage an occurrence comes
/// from (edit it once to change every page reusing it), and for a single
/// word, the near spellings the documentation has. Matching is literal and
/// ASCII-case-insensitive (byte offsets stay those of the text), on the
/// title and on `resolvedText` with its reuse markers taken out, or `text`
/// for a topic that reuses nothing.
fn find_text(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let query = arg_str(arguments, "text")?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if query.chars().count() < 2 {
        return Err(anyhow!("`text` must have at least 2 characters"));
    }
    let needle = query.to_ascii_lowercase();
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map_or(FIND_TEXT_PAGE, |v| v as usize);
    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let chunks = bundle.rag_chunks()?;
    if chunks.is_empty() {
        return Ok(
            "no rag/chunks.jsonl found for this bundle -- run `dita2graph-core build` \
             (not just `validate`) to produce one (§13.1)"
                .to_string(),
        );
    }

    struct Hit<'a> {
        chunk: &'a crate::bundle::RagChunk,
        resource_only: bool,
        count: usize,
        snippets: Vec<String>,
        /// The pages the topic pushes passages into (conref push), and how many.
        pushes: std::collections::BTreeMap<String, usize>,
    }
    let single_word = needle.len() >= 5
        && needle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    // word -> (occurrences, topics), for the near spellings of a single word.
    let mut words: std::collections::HashMap<String, (usize, std::collections::HashSet<&str>)> =
        std::collections::HashMap::new();
    // reused passage -> topics where an occurrence comes from it.
    let mut by_source: std::collections::BTreeMap<String, std::collections::BTreeSet<&str>> =
        std::collections::BTreeMap::new();
    let mut hits: Vec<Hit> = Vec::new();
    // Occurrences inside a draft-comment or required-cleanup.
    let mut in_notes = 0usize;
    // Occurrences in passages pushed into another page, and those pages.
    let mut in_pushes = 0usize;
    let mut pushed_into: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for chunk in chunks.iter() {
        let plain = PlainText::of(chunk);
        if single_word {
            for word in words_of(&plain.text) {
                let entry = words.entry(word).or_default();
                entry.0 += 1;
                entry.1.insert(chunk.id.as_str());
            }
        }
        let title_hit = chunk.title.to_ascii_lowercase().contains(&needle);
        let haystack = plain.text.to_ascii_lowercase();
        let starts: Vec<usize> = haystack
            .match_indices(&needle)
            .map(|(i, _)| i)
            .filter(|i| !plain.in_note_end(*i, i + needle.len()))
            .collect();
        if starts.is_empty() && !title_hit {
            continue;
        }
        let mut snippets = Vec::new();
        if title_hit {
            snippets.push(format!("in the title: {}", chunk.title));
        }
        for start in &starts {
            let end = start + needle.len();
            let source = plain.reused_source(*start, end);
            if let Some(source) = source {
                by_source
                    .entry(source.to_string())
                    .or_default()
                    .insert(chunk.id.as_str());
            }
            let note = plain.note_kind(*start, end);
            if note.is_some() {
                in_notes += 1;
            }
            let push = plain.pushed_label(*start, end);
            if let Some(label) = push {
                in_pushes += 1;
                pushed_into.insert(split_push_label(label).0.to_string());
            }
            if snippets.len() < FIND_TEXT_SNIPPETS {
                let mut line = context(&plain.text, *start, end);
                if let Some(source) = source {
                    line.push_str(&format!(" (in a passage reused from {source})"));
                }
                if let Some(label) = push {
                    line.push_str(&format!(
                        " (in a passage this topic pushes {label} with conref push: readers see \
                         it on that page)"
                    ));
                }
                if let Some(kind) = note {
                    line.push_str(&format!(
                        " (in a {kind}: a note for the authors, which the output leaves out \
                         unless the build sets args.draft)"
                    ));
                }
                snippets.push(line);
            }
        }
        let mut pushes = std::collections::BTreeMap::new();
        for (_, _, label) in &plain.pushed {
            *pushes
                .entry(split_push_label(label).0.to_string())
                .or_default() += 1;
        }
        hits.push(Hit {
            chunk,
            resource_only: !role_note(bundle, &chunk.id).is_empty(),
            count: starts.len() + usize::from(title_hit),
            snippets,
            pushes,
        });
    }
    hits.sort_by(|a, b| {
        a.resource_only
            .cmp(&b.resource_only)
            .then_with(|| {
                a.chunk
                    .title
                    .to_lowercase()
                    .cmp(&b.chunk.title.to_lowercase())
            })
            .then_with(|| a.chunk.id.cmp(&b.chunk.id))
    });

    let searched = "every topic's text was searched (titles, summaries and bodies, reused \
                    passages included; ASCII letters case-insensitive)";
    // "Complete" is about this text, not about the question: a complete
    // search for TODO and FIXME that found nothing was read as "no page has
    // a note for its authors", while one did, in other words (R7).
    let mut out = if hits.is_empty() {
        format!(
            "`{query}` appears in no topic: {searched}. This covers this exact text only, not \
             other wordings: search them before concluding that something is absent."
        )
    } else {
        let occurrences: usize = hits.iter().map(|h| h.count).sum();
        let published = hits.iter().filter(|h| !h.resource_only).count();
        format!(
            "`{query}`: {} in {} -- {published} published, {} resource-only (not pages of \
             the publication). This list is complete for this exact text, not for other \
             wordings of the same thing: {searched}.",
            count_of(occurrences, "occurrence"),
            count_of(hits.len(), "topic"),
            hits.len() - published,
        )
    };
    if in_notes > 0 {
        out.push_str(&format!(
            "\n{} in draft-comment or required-cleanup elements: notes for the authors, \
             which the output leaves out unless the build sets args.draft.",
            if in_notes == 1 {
                "1 of them is".to_string()
            } else {
                format!("{in_notes} of them are")
            }
        ));
    }
    if in_pushes > 0 {
        let pages: Vec<String> = pushed_into
            .iter()
            .map(|file| format!("{file}{}", push_target_note(bundle, file)))
            .collect();
        out.push_str(&format!(
            "\n{} in passages pushed into another page with conref push: readers see them on \
             that page, even when the topic holding them is resource-only. Pushed into: {}.",
            if in_pushes == 1 {
                "1 of them is".to_string()
            } else {
                format!("{in_pushes} of them are")
            },
            pages.join("; ")
        ));
    }
    let missing_shown = pushed_into
        .iter()
        .chain(
            hits.iter()
                .skip(offset)
                .take(limit)
                .flat_map(|h| h.pushes.keys()),
        )
        .any(|file| push_target_note(bundle, file).ends_with(MISSING_NOTE));
    if missing_shown {
        out.push_str(&format!("\n{MISSING_EXPLAINED}"));
    }
    if let Some(keys) = key_definitions(bundle, &needle) {
        out.push_str(&keys);
    }
    if !by_source.is_empty() {
        let sources: Vec<String> = by_source
            .iter()
            .map(|(source, topics)| format!("{source} (in {})", count_of(topics.len(), "topic")))
            .collect();
        out.push_str(&format!(
            "\nReused passages that hold it: {}. Editing a passage changes every topic that reuses it.",
            sources.join("; ")
        ));
    }
    if single_word {
        let variants = near_spellings(&needle, &words);
        if !variants.is_empty() {
            let listed: Vec<String> = variants
                .iter()
                .map(|(word, occurrences, topics)| {
                    format!(
                        "{word} ({} in {})",
                        count_of(*occurrences, "occurrence"),
                        count_of(*topics, "topic")
                    )
                })
                .collect();
            out.push_str(&format!(
                "\nNear spellings in the documentation, not counted above: {}. Search them \
                 too when the question is about every spelling.",
                listed.join(", ")
            ));
        }
    }
    if hits.is_empty() || limit == 0 {
        return Ok(out);
    }
    let end = (offset + limit).min(hits.len());
    for hit in hits.iter().take(end).skip(offset) {
        out.push_str(&format!(
            "\n\n{} ({}) [{}]{}: {}",
            hit.chunk.title,
            hit.chunk.topic_type,
            hit.chunk.id,
            role_note(bundle, &hit.chunk.id),
            count_of(hit.count, "occurrence")
        ));
        for snippet in &hit.snippets {
            out.push_str(&format!("\n  {snippet}"));
        }
        if hit.count > hit.snippets.len() {
            out.push_str(&format!(
                "\n  (+{} more in this topic)",
                hit.count - hit.snippets.len()
            ));
        }
        // A warehouse whose entries are pushed into other pages: the text
        // next to an occurrence may reach readers there (bench r-d6).
        if !hit.pushes.is_empty() {
            let pages: Vec<String> = hit
                .pushes
                .iter()
                .map(|(file, n)| format!("{n} into {file}{}", push_target_note(bundle, file)))
                .collect();
            out.push_str(&format!(
                "\n  This topic pushes passages into other pages with conref push, where readers \
                 see them: {}. read_topic shows each one between [pushed ...] and [end pushed].",
                pages.join("; ")
            ));
        }
    }
    if offset > 0 || end < hits.len() {
        out.push_str(&format!(
            "\n\n(showing {}-{end} of {} topics{})",
            (offset + 1).min(end),
            hits.len(),
            if end < hits.len() {
                format!(": pass offset={end} for the next page")
            } else {
                String::new()
            }
        ));
    }
    Ok(out)
}

/// At most this many keys named by [`key_definitions`].
const FIND_TEXT_KEYS: usize = 10;

/// Where keys define `needle` (ASCII-lowercased), for [`find_text`]: the
/// keys whose link text holds it, and those targeting a URL that holds it,
/// each with the map that defines it; then the URLs holding it that no key
/// targets. Found by the relevance checks (R5): a literal search for the
/// plug-in registry's address listed the passages that show it, and the
/// agent told the user to edit them by hand, never naming the key in
/// resources/external-links.ditamap that puts the address in every one.
fn key_definitions(bundle: &BundleReader, needle: &str) -> Option<String> {
    use dita2graph_core::graph_view::{EXTERNAL_TYPE, KEY_TYPE};
    // key name -> (defining map, what of it holds the text)
    let mut keys: std::collections::BTreeMap<String, (String, Vec<String>)> =
        std::collections::BTreeMap::new();
    let mut direct_urls: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut add = |key_id: &str, what: String| {
        let name = key_id.strip_prefix("key:").unwrap_or(key_id).to_string();
        let file = bundle
            .node(key_id)
            .and_then(|key| key.source_file.clone())
            .unwrap_or_default();
        keys.entry(name)
            .or_insert_with(|| (file, Vec::new()))
            .1
            .push(what);
    };
    for node in bundle.all_nodes() {
        if node.type_ == KEY_TYPE {
            // A key's title is `name = "link text"`, or its name alone.
            let text = node
                .title
                .as_deref()
                .and_then(|title| title.split_once(" = "))
                .map(|(_, text)| text.trim_matches('"'));
            if let Some(text) = text
                && text.to_ascii_lowercase().contains(needle)
            {
                add(&node.id, format!("its link text \"{text}\""));
            }
        } else if node.type_ == EXTERNAL_TYPE && node.id.to_ascii_lowercase().contains(needle) {
            let targeting = bundle.edges_to(&node.id, Some("key-target"));
            if targeting.is_empty() {
                direct_urls.insert(&node.id);
            }
            for edge in targeting {
                add(&edge.from, format!("its target {}", node.id));
            }
        }
    }
    let mut out = String::new();
    if !keys.is_empty() {
        let named: Vec<String> = keys
            .iter()
            .take(FIND_TEXT_KEYS)
            .map(|(name, (file, what))| {
                let file = if file.is_empty() {
                    String::new()
                } else {
                    format!(" in {file}")
                };
                format!("`{name}`{file} ({})", what.join("; "))
            })
            .collect();
        let more = keys.len().saturating_sub(FIND_TEXT_KEYS);
        out.push_str(&format!(
            "\nKeys define it: {}{}. A page that shows it through a key changes when the key's \
             definition changes, not its own text; analyze_impact on key:<name> lists those pages.",
            named.join("; "),
            if more > 0 {
                format!(" (+{more} more keys)")
            } else {
                String::new()
            }
        ));
    }
    if !direct_urls.is_empty() {
        let shown: Vec<&str> = direct_urls.iter().take(FIND_TEXT_KEYS).copied().collect();
        out.push_str(&format!(
            "\nURLs that hold it, linked without a key: {}; analyze_impact on a URL lists the \
             pages linking to it.",
            shown.join(", ")
        ));
    }
    (!out.is_empty()).then_some(out)
}

/// `1 topic`, `2 topics`.
fn count_of(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// A topic's text as [`find_text`] searches it: the reuse and push markers
/// of `resolvedText` taken out, whitespace runs made one space, and for
/// each reused or pushed passage the byte range it covers and where it
/// comes from or goes.
struct PlainText {
    text: String,
    /// `(start, end, "<file>#<fragment>")`, a nested passage after the one holding it.
    reused: Vec<(usize, usize, String)>,
    /// `(start, end, "after <file>#<element>")` of each passage the topic
    /// pushes into another with conref push, from its `[pushed ...]` marker
    /// to the end of its `[end pushed]` one, which the extractor writes.
    pushed: Vec<(usize, usize, String)>,
    /// `(start, end, kind)` of each note for the authors, from its
    /// `[draft-comment]` marker to the end of its `[end draft-comment]` one
    /// (`required-cleanup` the same way), which the extractor writes.
    notes: Vec<(usize, usize, &'static str)>,
    /// The byte ranges of the end markers, so a search for `draft-comment`
    /// counts each note once, and of the push markers, whose file and
    /// element names are no occurrence either.
    note_ends: Vec<(usize, usize)>,
}

impl PlainText {
    const OPEN: &'static str = "[reused from ";
    const CLOSE: &'static str = "[end reused]";
    const PUSH_OPEN: &'static str = "[pushed ";
    const PUSH_CLOSE: &'static str = "[end pushed]";
    const NOTE_KINDS: [&'static str; 2] = ["draft-comment", "required-cleanup"];

    fn of(chunk: &crate::bundle::RagChunk) -> Self {
        Self::parse(
            chunk
                .resolved_text
                .as_deref()
                .or(chunk.text.as_deref())
                .unwrap_or_default(),
        )
    }

    fn parse(source: &str) -> Self {
        let mut text = String::with_capacity(source.len());
        let mut reused = Vec::new();
        let mut open: Vec<(usize, String)> = Vec::new();
        let mut rest = source;
        loop {
            let next_open = rest.find(Self::OPEN);
            let next_close = rest.find(Self::CLOSE);
            match (next_open, next_close) {
                (Some(o), c) if c.is_none_or(|c| o < c) => {
                    push_collapsed(&mut text, &rest[..o]);
                    let after = &rest[o + Self::OPEN.len()..];
                    let Some(end) = after.find(']') else {
                        push_collapsed(&mut text, &rest[o..]);
                        break;
                    };
                    open.push((text.len(), after[..end].to_string()));
                    rest = &after[end + 1..];
                }
                (_, Some(c)) => {
                    push_collapsed(&mut text, &rest[..c]);
                    if let Some((start, source)) = open.pop() {
                        reused.push((start, text.len(), source));
                    }
                    rest = &rest[c + Self::CLOSE.len()..];
                }
                _ => {
                    push_collapsed(&mut text, rest);
                    break;
                }
            }
        }
        while let Some((start, source)) = open.pop() {
            reused.push((start, text.len(), source));
        }
        let text = text.trim_end().to_string();
        let mut notes = Vec::new();
        let mut note_ends = Vec::new();
        for kind in Self::NOTE_KINDS {
            let open = format!("[{kind}]");
            let close = format!("[end {kind}]");
            let mut from = 0;
            while let Some(start) = text[from..].find(&open).map(|i| i + from) {
                let after = start + open.len();
                match text[after..].find(&close).map(|i| i + after) {
                    Some(end) => {
                        notes.push((start, end + close.len(), kind));
                        note_ends.push((end, end + close.len()));
                        from = end + close.len();
                    }
                    None => {
                        notes.push((start, text.len(), kind));
                        break;
                    }
                }
            }
        }
        // A pushed passage keeps its markers, as the notes do: an excerpt
        // then shows where the text next to an occurrence goes. A match
        // inside a marker is no occurrence.
        let mut pushed = Vec::new();
        let mut from = 0;
        while let Some(start) = text[from..].find(Self::PUSH_OPEN).map(|i| i + from) {
            let label_start = start + Self::PUSH_OPEN.len();
            let Some(label_end) = text[label_start..].find(']').map(|i| i + label_start) else {
                break;
            };
            note_ends.push((start, label_end + 1));
            let label = text[label_start..label_end].to_string();
            match text[label_end..]
                .find(Self::PUSH_CLOSE)
                .map(|i| i + label_end)
            {
                Some(end) => {
                    let close_end = end + Self::PUSH_CLOSE.len();
                    pushed.push((start, close_end, label));
                    note_ends.push((end, close_end));
                    from = close_end;
                }
                None => {
                    pushed.push((start, text.len(), label));
                    break;
                }
            }
        }
        PlainText {
            text,
            reused,
            pushed,
            notes,
            note_ends,
        }
    }

    /// The label (`after <file>#<element>`) of the innermost passage the
    /// topic pushes into another that holds `start..end`, if any.
    fn pushed_label(&self, start: usize, end: usize) -> Option<&str> {
        self.pushed
            .iter()
            .filter(|(s, e, _)| *s <= start && end <= *e)
            .min_by_key(|(s, e, _)| e - s)
            .map(|(_, _, label)| label.as_str())
    }

    /// The kind of the note for the authors that holds `start..end`, if any.
    fn note_kind(&self, start: usize, end: usize) -> Option<&'static str> {
        self.notes
            .iter()
            .find(|(s, e, _)| *s <= start && end <= *e)
            .map(|(_, _, kind)| *kind)
    }

    /// Whether `start..end` overlaps an end marker of a note.
    fn in_note_end(&self, start: usize, end: usize) -> bool {
        self.note_ends.iter().any(|(s, e)| start < *e && *s < end)
    }

    /// The innermost reused passage that holds `start..end`, if any.
    fn reused_source(&self, start: usize, end: usize) -> Option<&str> {
        self.reused
            .iter()
            .filter(|(s, e, _)| *s <= start && end <= *e)
            .min_by_key(|(s, e, _)| e - s)
            .map(|(_, _, source)| source.as_str())
    }
}

/// Appends `s` with every run of whitespace made one space, and none at the
/// start of the text or after a space already there.
fn push_collapsed(out: &mut String, s: &str) {
    for c in s.chars() {
        if c.is_whitespace() {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

/// `text[start..end]` with up to [`FIND_TEXT_CONTEXT`] bytes on each side,
/// cut on character boundaries, `…` where the text goes on.
fn context(text: &str, start: usize, end: usize) -> String {
    let mut from = start.saturating_sub(FIND_TEXT_CONTEXT);
    while !text.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (end + FIND_TEXT_CONTEXT).min(text.len());
    while !text.is_char_boundary(to) {
        to += 1;
    }
    format!(
        "{}{}{}",
        if from > 0 { "…" } else { "" },
        &text[from..to],
        if to < text.len() { "…" } else { "" }
    )
}

/// The words of `text`, ASCII-lowercased: runs of letters, digits, `_`,
/// `-` and `.`, without the `.`, `-` or `_` they start or end with, three
/// characters or more.
fn words_of(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .map(|w| w.trim_matches(|c| matches!(c, '_' | '-' | '.')))
        .filter(|w| w.chars().count() >= 3)
        .map(str::to_ascii_lowercase)
}

/// The words of the documentation that look like a misspelling of `needle`
/// (its `-`, `_` and `.` at either end left out, as [`words_of`] does): one
/// edit away, or two with the same letters moved -- `pluginsdir` for
/// `plugindirs` -- but not a word that contains it or that it contains
/// (`installs`, `uninstall` for `install`), nor two letters changed for two
/// others (`conference` for `confluence`). Closest and most frequent first,
/// at most ten: `(word, occurrences, topics)`.
fn near_spellings(
    needle: &str,
    words: &std::collections::HashMap<String, (usize, std::collections::HashSet<&str>)>,
) -> Vec<(String, usize, usize)> {
    let core = needle.trim_matches(|c| matches!(c, '_' | '-' | '.'));
    let letters = |w: &str| {
        let mut chars: Vec<char> = w.chars().collect();
        chars.sort_unstable();
        chars
    };
    let core_letters = letters(core);
    let mut found: Vec<(usize, String, usize, usize)> = words
        .iter()
        .filter(|(word, _)| !word.contains(core) && !core.contains(word.as_str()))
        .filter_map(|(word, (occurrences, topics))| {
            let distance = edit_distance(word, core, 2)?;
            (distance == 1 || letters(word) == core_letters)
                .then(|| (distance, word.clone(), *occurrences, topics.len()))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)).then(a.1.cmp(&b.1)));
    found
        .into_iter()
        .take(10)
        .map(|(_, word, occurrences, topics)| (word, occurrences, topics))
        .collect()
}

/// The Levenshtein distance between `a` and `b` (by character), when it is
/// at most `max`.
fn edit_distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        if current.iter().min().is_some_and(|m| *m > max) {
            return None;
        }
        previous = current;
    }
    let distance = previous[b.len()];
    (distance <= max).then_some(distance)
}

/// Keyword-frequency relevance score across `terms` (already
/// lowercased, whitespace-split): a term appearing in the title counts
/// more than one appearing in the body (title relevance is a stronger
/// signal than an incidental mention), and every occurrence in the body
/// adds to the score, so a term mentioned five times outranks one
/// mentioned once. This is word-overlap/term-frequency ranking, not
/// embedding-based semantic similarity -- §13.1 is explicit that the
/// latter is a separate, heavier, not-yet-committed step, and this
/// stays a keyword-matching improvement on top of the plain substring
/// check it replaces, not a step toward it.
fn relevance_score(terms: &[&str], title_lower: &str, text_lower: &str) -> i64 {
    let mut score = 0i64;
    for term in terms {
        if term.is_empty() {
            continue;
        }
        if title_lower.contains(term) {
            score += 5;
        }
        score += text_lower.matches(term).count() as i64;
    }
    score
}

/// Every id reachable from `start` by following outgoing edges
/// (optionally restricted to one `relation`) up to `depth` hops,
/// including `start` itself -- the graph-narrowing step in
/// [`search_content`].
fn forward_reachable(
    bundle: &BundleReader,
    start: &str,
    relation: Option<&str>,
    depth: usize,
) -> std::collections::HashSet<String> {
    let mut visited = std::collections::HashSet::new();
    visited.insert(start.to_string());
    let mut frontier = vec![start.to_string()];

    for _ in 0..depth {
        let mut next = Vec::new();
        for id in &frontier {
            for edge in bundle.edges_from(id, relation) {
                if visited.insert(edge.to.clone()) {
                    next.push(edge.to.clone());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    visited
}

/// Default `maxChars` for [`read_topic`]: a long reference topic stays
/// readable in one call, while a pathological one can't flood the client.
const READ_TOPIC_MAX_CHARS: usize = 20_000;

/// A topic's DITA metadata as `read_topic` shows it, one line per kind and
/// nothing for what the topic does not have: its dates, permissions,
/// products, audiences, authors, keywords and index entries
/// (`docs/semantic-model.md`, "DITA metadata").
fn metadata_lines(m: &dita2graph_core::Metadata) -> String {
    let mut lines = Vec::new();
    let dates: Vec<String> = [
        ("created", &m.created),
        ("modified", &m.modified),
        ("go-live", &m.golive),
        ("expiry", &m.expiry),
        ("permissions", &m.permissions),
    ]
    .iter()
    .filter_map(|(label, value)| value.as_ref().map(|v| format!("{label} {v}")))
    .collect();
    if !dates.is_empty() {
        lines.push(format!("Dates: {}", dates.join(", ")));
    }
    if !m.products.is_empty() {
        let products: Vec<String> = m
            .products
            .iter()
            .map(|p| match &p.version {
                Some(version) => format!("{} {version}", p.name),
                None => p.name.clone(),
            })
            .collect();
        lines.push(format!("Product: {}", products.join(", ")));
    }
    if !m.audiences.is_empty() {
        let audiences: Vec<String> = m
            .audiences
            .iter()
            .map(|a| {
                [&a.type_, &a.job, &a.experience_level]
                    .iter()
                    .filter_map(|v| v.as_deref())
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .collect();
        lines.push(format!("Audience: {}", audiences.join(", ")));
    }
    if !m.authors.is_empty() {
        let authors: Vec<String> = m
            .authors
            .iter()
            .map(|a| match &a.type_ {
                Some(type_) => format!("{} ({type_})", a.name),
                None => a.name.clone(),
            })
            .collect();
        lines.push(format!("Authors: {}", authors.join(", ")));
    }
    if !m.keywords.is_empty() {
        lines.push(format!("Keywords: {}", m.keywords.join(", ")));
    }
    if !m.index_terms.is_empty() {
        // An add-on catalogue page of the Oxygen guide has 27 entries: the
        // first ones say what the topic is indexed under; the rest is in
        // the concept file and the RDF export.
        const SHOWN: usize = 20;
        let entries: Vec<String> = m
            .index_terms
            .iter()
            .take(SHOWN)
            .map(|path| path.join(" > "))
            .collect();
        let more = m.index_terms.len().saturating_sub(SHOWN);
        lines.push(format!(
            "Index: {}{}",
            entries.join("; "),
            if more > 0 {
                format!(" (+{more} more)")
            } else {
                String::new()
            }
        ));
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// One topic's whole text, reused fragments included. Found by the
/// token/latency benchmark (dita2graph-demo/bench, q1/q2): with only
/// `explain_task`'s 300-character excerpt and `search_content`'s
/// 200-character ones, an agent made 13 to 26 calls without ever seeing
/// the sentence it needed, while one `Read` of the source file sufficed.
/// Reads the chunk's `resolvedText` (fragments in place, between markers)
/// and falls back to its deduplicated `text` for a topic that reuses
/// nothing, or for a bundle built before `resolvedText` existed.
fn read_topic(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?;
    let max_chars = arguments
        .get("maxChars")
        .and_then(|v| v.as_u64())
        .map_or(READ_TOPIC_MAX_CHARS, |v| v as usize);
    if bundle
        .node(topic_id)
        .is_some_and(|n| n.type_ == MISSING_TYPE)
    {
        return Ok(missing_file(bundle, topic_id));
    }
    let (frontmatter, _) = bundle.read_concept(topic_id)?;
    let field = |key: &str| {
        frontmatter
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let title = field("title").unwrap_or_else(|| topic_id.to_string());
    let mut out = format!(
        "{title} ({}) [{topic_id}]{}\nSource: {}\n",
        field("type").unwrap_or_else(|| "?".into()),
        role_note(bundle, topic_id),
        field("resource").unwrap_or_else(|| "?".into())
    );
    out.push_str(&metadata_lines(&bundle.metadata(topic_id)));

    let chunks = bundle.rag_chunks().unwrap_or_default();
    let text = chunks
        .iter()
        .find(|c| c.id == topic_id)
        .and_then(|c| c.resolved_text.as_deref().or(c.text.as_deref()));
    match text {
        Some(text) => {
            let total = text.chars().count();
            out.push('\n');
            out.extend(text.chars().take(max_chars));
            if total > max_chars {
                out.push_str(&format!(
                    "\n\n(truncated at {max_chars} of {total} characters -- pass a larger maxChars to read the rest)"
                ));
            }
            out.push('\n');
        }
        None => out.push_str("\n(no body text for this topic)\n"),
    }

    // The passages pushed into other pages, by page: a resource-only
    // warehouse's text reaches readers there (bench r-d6).
    let mut pushes: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (_, _, label) in text
        .map(PlainText::parse)
        .map(|p| p.pushed)
        .unwrap_or_default()
    {
        let (file, how) = split_push_label(&label);
        pushes.entry(file.to_string()).or_default().push(how);
    }
    if !pushes.is_empty() {
        out.push_str(
            "\nPushed into other pages with conref push (readers see these passages there), by \
             page:\n",
        );
        let mut missing = false;
        for (file, hows) in &pushes {
            let shown: Vec<&str> = hows.iter().take(PUSHES_SHOWN).map(String::as_str).collect();
            let note = push_target_note(bundle, file);
            missing |= note.ends_with(MISSING_NOTE);
            out.push_str(&format!(
                "  - {file}{note}: {} passage(s), {}{}\n",
                hows.len(),
                shown.join(", "),
                if hows.len() > PUSHES_SHOWN {
                    format!(" (+{} more)", hows.len() - PUSHES_SHOWN)
                } else {
                    String::new()
                }
            ));
        }
        if missing {
            out.push_str(&format!("{MISSING_EXPLAINED}\n"));
        }
    }

    let reused = bundle.edges_from(topic_id, Some("generated-from"));
    if !reused.is_empty() {
        out.push_str(
            "\nReused content (conref/conkeyref, or pushed into this topic with conref push), by \
             source topic:\n",
        );
        for edge in reused {
            let source_title = bundle.title(&edge.to).unwrap_or_else(|_| edge.to.clone());
            let resource = bundle
                .read_concept(&edge.to)
                .ok()
                .and_then(|(fm, _)| {
                    fm.get("resource")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "  - {source_title} ({}, {resource}){}",
                edge.to,
                role_note(bundle, &edge.to)
            ));
            if !edge.fragments.is_empty() {
                out.push_str(&format!(": {}", edge.fragments.join(", ")));
            }
            out.push('\n');
        }
    }
    for (relation, heading) in [
        ("links-to", "External links"),
        (
            "uses-key",
            "Keys used (key -> what it names, defining file)",
        ),
        ("includes-code", "Sample files included (coderef)"),
    ] {
        let edges = bundle.edges_from(topic_id, Some(relation));
        if edges.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{heading}:\n"));
        for edge in edges {
            out.push_str(&format!("  - {}", edge.to));
            if relation == "uses-key" {
                if let Some(target) = bundle.edges_from(&edge.to, Some("key-target")).first() {
                    out.push_str(&format!(" -> {}", target.to));
                }
                // A text key's value: its node is titled `name = "text"`.
                if let Some((_, value)) = bundle
                    .title(&edge.to)
                    .ok()
                    .as_deref()
                    .and_then(|t| t.split_once(" = "))
                {
                    out.push_str(&format!(" = {value}"));
                }
                if let Some(file) = bundle.node(&edge.to).and_then(|n| n.source_file.as_deref()) {
                    out.push_str(&format!(" ({file})"));
                } else {
                    out.push_str(" (defined in no map)");
                }
            }
            out.push('\n');
        }
    }
    Ok(out)
}

/// At most this many pushed passages listed per page by [`read_topic`].
const PUSHES_SHOWN: usize = 12;

/// [`read_topic`] for a topic file the build does not have: what the graph
/// knows of it -- the keys that name it and who links through them, and
/// what other topics push into it. Without this, a link through such a key
/// read as an empty link, and a passage pushed into the page as one no
/// reader sees (bench r-d6).
fn missing_file(bundle: &BundleReader, id: &str) -> String {
    let path = bundle.title(id).unwrap_or_else(|_| id.to_string());
    let mut out = format!(
        "{path} [{id}]: not in this build. DITA-OT could not load this file: it is a page the \
         documentation's own build generates before running DITA-OT, or a missing file. The \
         graph has no text of its own for it. If the publication's build does generate it, \
         readers see it, with what other topics push into it.\n"
    );
    let keys = bundle.edges_to(id, Some("key-target"));
    if !keys.is_empty() {
        out.push_str("\nKeys that name it (and the topics that use each key):\n");
        for edge in keys {
            let file = bundle
                .node(&edge.from)
                .and_then(|n| n.source_file.clone())
                .map(|file| format!(", defined in {file}"))
                .unwrap_or_default();
            let mut users: Vec<String> = bundle
                .edges_to(&edge.from, Some("uses-key"))
                .iter()
                .map(|use_| describe_node(bundle, &use_.from))
                .collect();
            users.sort();
            out.push_str(&format!(
                "  - {}{file}: {}\n",
                edge.from,
                if users.is_empty() {
                    "no topic uses it".to_string()
                } else {
                    users.join("; ")
                }
            ));
        }
    }
    let pushers = bundle.edges_from(id, Some("generated-from"));
    if !pushers.is_empty() {
        out.push_str("\nPushed into it with conref push, by topic (the fragments it pushes):\n");
        for edge in pushers {
            out.push_str(&format!(
                "  - {}{}\n",
                describe_node(bundle, &edge.to),
                fragments_suffix(edge)
            ));
        }
        out.push_str(
            "read_topic on a pushing topic shows each pushed passage between [pushed ...] and \
             [end pushed], and where it goes.\n",
        );
    }
    out
}

/// ` [resource-only: not a published page]` after a topic the map uses
/// only as a resource -- a reuse warehouse, a key target -- empty for any
/// other node. Without it, an agent listing the topics that link to a URL
/// counted the warehouse holding a reused link as one more published page
/// (dita2graph-demo/bench, campaign 4 replay, question r-o8).
///
/// A topic file the build does not have gets [`MISSING_NOTE`] instead:
/// without it, an agent read a link through a key naming such a file --
/// a page the documentation's own build generates -- as an empty link
/// (bench r-d6).
fn role_note(bundle: &BundleReader, id: &str) -> &'static str {
    match bundle.node(id) {
        Some(node) if node.processing_role.as_deref() == Some("resource-only") => {
            " [resource-only: not a published page]"
        }
        Some(node) if node.type_ == MISSING_TYPE => MISSING_NOTE,
        _ => "",
    }
}

/// [`role_note`] of a topic file the build does not have; a list that
/// shows it adds [`MISSING_EXPLAINED`] once.
const MISSING_NOTE: &str = " [not in this build]";

/// What [`MISSING_NOTE`] means.
const MISSING_EXPLAINED: &str = "A file marked [not in this build] is one DITA-OT could not \
                                 load: a page the documentation's own build generates before \
                                 running DITA-OT, or a missing file. The graph has no text of its \
                                 own for it; read_topic on it says which keys name it and what is \
                                 pushed into it.";

/// What `[pushed <label>]` names: the file a passage is pushed into, and
/// how (`after <element>`, `before <element>`, `to replace <element>`).
fn split_push_label(label: &str) -> (&str, String) {
    let (action, target) = ["after ", "before ", "to replace "]
        .iter()
        .find_map(|action| label.strip_prefix(action).map(|t| (action.trim_end(), t)))
        .unwrap_or(("into", label));
    match target.split_once('#') {
        Some((file, element)) => (file, format!("{action} {element}")),
        None => (target, action.to_string()),
    }
}

/// `, <topic>` for a file a topic is pushed into that is a topic of the
/// build, [`MISSING_NOTE`] for one it does not have, empty otherwise.
fn push_target_note(bundle: &BundleReader, file: &str) -> String {
    let wanted = normalize_path(file);
    if let Some(topic) = bundle
        .source_files()
        .iter()
        .find(|f| normalize_path(&f.path) == wanted)
    {
        return format!(", {}", describe_node(bundle, &topic.id));
    }
    let missing = dita2graph_core::graph_view::file_id(file);
    if bundle
        .node(&missing)
        .is_some_and(|n| n.type_ == MISSING_TYPE)
    {
        return format!(" ({missing}){MISSING_NOTE}");
    }
    String::new()
}

/// ` [fragments: a, b]` for an edge that names the reused elements
/// (generated-from), empty otherwise.
fn fragments_suffix(edge: &crate::bundle::GraphEdge) -> String {
    if edge.fragments.is_empty() {
        String::new()
    } else {
        format!(" [fragments: {}]", edge.fragments.join(", "))
    }
}

fn find_related_topics(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?;
    let relation = arguments.get("relation").and_then(|v| v.as_str());
    let mut lines: Vec<String> = bundle
        .edges_from(topic_id, relation)
        .iter()
        .map(|e| {
            let title = bundle.title(&e.to).unwrap_or_else(|_| e.to.clone());
            format!(
                "{} -> {} ({}){}{}",
                e.relation,
                title,
                e.to,
                role_note(bundle, &e.to),
                fragments_suffix(e)
            )
        })
        .collect();
    // A key, URL or sample file has few edges of its own: what matters is
    // who points at it, and for a key, the file that defines it.
    if let Some(node) = bundle.node(topic_id).filter(|n| n.is_declared()) {
        if node.type_ == MISSING_TYPE {
            lines.insert(0, format!("{topic_id}{MISSING_NOTE}. {MISSING_EXPLAINED}"));
        } else if let Some(file) = &node.source_file {
            lines.insert(0, format!("{} {topic_id}, in {file}", node.type_));
        }
        for e in bundle.edges_to(topic_id, relation) {
            let title = bundle.title(&e.from).unwrap_or_else(|_| e.from.clone());
            lines.push(format!(
                "{} <- {} ({}){}",
                e.relation,
                title,
                e.from,
                role_note(bundle, &e.from)
            ));
        }
    }
    if lines.is_empty() {
        return Ok(format!("no relations found for `{topic_id}`"));
    }
    Ok(lines.join("\n"))
}

fn explain_task(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?;
    let (frontmatter, _body) = bundle.read_concept(topic_id)?;
    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(topic_id);
    let description = frontmatter
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("(no description)");

    let mut out = format!(
        "Topic: {title} ({topic_id}){}\n{description}\n",
        role_note(bundle, topic_id)
    );
    // Found live: a real Claude Code session asked what a task actually
    // covers and had no way to see its own body text through this tool
    // at all -- description above is just the shortdesc (one sentence,
    // often absent), and `_body` from read_concept is the *rendered*
    // concept file (its own "# Summary"/"# Content" headings included),
    // not clean prose worth excerpting directly. `rag/chunks.jsonl`
    // already holds this topic's clean body text -- the same source
    // `search_content`/`analyze_impact` excerpt from -- so reuse that
    // instead of re-deriving anything from the rendered markdown.
    if let Some(chunk) = bundle
        .rag_chunks()
        .unwrap_or_default()
        .iter()
        .find(|c| c.id == topic_id)
        && let Some(text) = chunk.text.as_deref()
    {
        out.push_str(&format!("\n{}\n", excerpt(text, 300)));
    }
    for relation in ["requires", "contains", "applies-to"] {
        let edges = bundle.edges_from(topic_id, Some(relation));
        if !edges.is_empty() {
            out.push_str(&format!("\n{}:\n", relation));
            for e in edges {
                let target_title = bundle.title(&e.to).unwrap_or_else(|_| e.to.clone());
                out.push_str(&format!(
                    "  - {target_title} ({}){}\n",
                    e.to,
                    role_note(bundle, &e.to)
                ));
            }
        }
    }
    Ok(out)
}

fn trace_dependencies(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?.to_string();
    let depth = arguments.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

    let mut visited = std::collections::HashSet::new();
    let mut frontier = vec![topic_id.clone()];
    let mut lines = Vec::new();
    visited.insert(topic_id);

    for level in 1..=depth {
        let mut next = Vec::new();
        for id in &frontier {
            for edge in bundle.edges_from(id, Some("requires")) {
                if visited.insert(edge.to.clone()) {
                    let title = bundle.title(&edge.to).unwrap_or_else(|_| edge.to.clone());
                    lines.push(format!(
                        "{}{} requires {} ({}){}",
                        "  ".repeat(level - 1),
                        id,
                        title,
                        edge.to,
                        role_note(bundle, &edge.to)
                    ));
                    next.push(edge.to.clone());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    if lines.is_empty() {
        return Ok("no `requires` dependencies found".to_string());
    }
    Ok(lines.join("\n"))
}

/// Reverse BFS over every relation, not just `requires` like
/// [`trace_dependencies`] -- "if I change this topic, what breaks?"
/// needs `contains` (which maps/topics include it) and `references` too,
/// not only declared prerequisites (§13.1). Each affected concept gets a
/// text excerpt from `rag/chunks.jsonl` under it when one exists -- the
/// "content layer summarizes the impact" half of §13.1's design text,
/// implemented as a raw excerpt handed to the calling agent rather than
/// a server-generated summary (this tool doesn't call an LLM); the
/// agent's own read of the excerpts is the summary.
///
/// Only a copy, a key or an included file spreads further than one hop
/// (see [`Reach`]): following every relation to `depth` 5 counted 112
/// pages for the DITA-OT docs' "Installing plug-ins" page, where a writer
/// found about twenty to rewrite. The rest only linked to, sat in a map
/// with, or reused another part of a page that depends on it -- every
/// page that reuses any fragment of the conref warehouse holding its
/// link, among them. Those are counted apart, and `cascade` lists them.
///
/// Paginated, with the totals up front: on the Oxygen XML user guide
/// (2,040 topics), the unpaginated result for its most reused conref
/// warehouse ran to 613 entries, more than an MCP client displays inline.
/// Claude Code saved it to a file the agent had no tool to read, and the
/// answer stopped at a lower bound (dita2graph-demo/bench, question o8).
/// The first line now gives the total, by relation and by level, which
/// answers "how many" on its own; `relation`/`fragment` narrow the
/// traversal ("who reuses fragment X" is `relation`-free, `fragment`=X,
/// `depth`=1); `limit`/`offset` page through the rest.
fn analyze_impact(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?.to_string();
    let depth = arguments.get("depth").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
    let relation = arguments.get("relation").and_then(|v| v.as_str());
    let fragment = arguments.get("fragment").and_then(|v| v.as_str());
    let cascade = arguments
        .get("cascade")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map_or(IMPACT_PAGE, |v| v as usize);
    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let with_excerpts = arguments
        .get("excerpts")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let affected = impact_hits(bundle, &topic_id, depth, relation, fragment, false);
    // Following every relation reaches a superset: what it adds only
    // points at an affected concept.
    let affected_ids: std::collections::HashSet<&str> = affected
        .iter()
        .map(|(_, edge, _)| edge.from.as_str())
        .collect();
    let pointing: Vec<_> = impact_hits(bundle, &topic_id, depth, relation, fragment, true)
        .into_iter()
        .filter(|(_, edge, _)| !affected_ids.contains(edge.from.as_str()))
        .collect();

    let mut scope = vec![format!("depth {depth}")];
    if let Some(relation) = relation {
        scope.push(format!("relation {relation}"));
    }
    if let Some(fragment) = fragment {
        scope.push(format!("fragment {fragment}"));
    }
    if cascade {
        scope.push("cascade".to_string());
    }
    let scope = scope.join(", ");
    if affected.is_empty() {
        return Ok(format!(
            "nothing depends on `{topic_id}` ({scope}: no incoming edges found)"
        ));
    }

    // The totals count concepts (maps and topics): a key, URL or file
    // reached on the way is listed with them but is no page.
    let is_concept = |id: &str| !bundle.node(id).is_some_and(|n| n.is_declared());
    let mut concepts = 0;
    let mut by_relation: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut by_level: std::collections::BTreeMap<usize, usize> = Default::default();
    for (level, edge, _) in &affected {
        if !is_concept(&edge.from) {
            continue;
        }
        concepts += 1;
        *by_relation.entry(edge.relation.as_str()).or_default() += 1;
        *by_level.entry(*level).or_default() += 1;
    }
    let join = |parts: Vec<String>| parts.join(", ");
    let mut out = format!(
        "changing `{topic_id}` may affect {concepts} concept(s) ({scope}). By relation: {}. By level: {}.",
        join(
            by_relation
                .iter()
                .map(|(r, n)| format!("{r} {n}"))
                .collect()
        ),
        join(by_level.iter().map(|(l, n)| format!("{l}: {n}")).collect())
    );
    let declared = affected.len() - concepts;
    if declared > 0 {
        out.push_str(&format!(
            " {declared} key, URL or file node(s) on the way are listed too, not counted."
        ));
    }
    let missing = affected
        .iter()
        .filter(|(_, edge, _)| {
            bundle
                .node(&edge.from)
                .is_some_and(|n| n.type_ == MISSING_TYPE)
        })
        .count();
    if missing > 0 {
        out.push_str(&format!(
            " {missing} of them are pages this build does not have (marked [not in this build]), \
             which content is pushed into or a key names: when the publication's own build \
             generates them, their readers see the change too."
        ));
    }
    let pointing_concepts = pointing
        .iter()
        .filter(|(_, edge, _)| is_concept(&edge.from))
        .count();
    if pointing_concepts > 0 {
        out.push_str(&format!(
            " {pointing_concepts} more concept(s) are only connected to these (links, maps, keys \
             or reuse of their other parts), their own content unchanged: {}.",
            if cascade {
                "listed after them"
            } else {
                "pass cascade=true to list them"
            }
        ));
    }
    let hits: Vec<_> = if cascade {
        affected.iter().chain(&pointing).collect()
    } else {
        affected.iter().collect()
    };
    let total = hits.len();
    if limit == 0 {
        return Ok(out);
    }
    if offset >= total {
        out.push_str(&format!(
            "\n(offset {offset} is past the last of the {total} entries)"
        ));
        return Ok(out);
    }

    // Best-effort enrichment: a missing or malformed rag/ shouldn't break
    // the graph traversal, which is this tool's primary job.
    let chunks = if with_excerpts {
        bundle.rag_chunks().unwrap_or_default()
    } else {
        Default::default()
    };
    let excerpt_by_id: std::collections::HashMap<&str, String> = chunks
        .iter()
        .filter_map(|c| c.text.as_deref().map(|t| (c.id.as_str(), excerpt(t, 140))))
        .collect();

    let end = offset.saturating_add(limit).min(total);
    for (index, (level, edge, id)) in hits.iter().enumerate().take(end).skip(offset) {
        // The cascade's heading, on every page that holds some of it.
        if index == affected.len().max(offset) {
            out.push_str(
                "\n\nOnly connected to the concepts above (links, maps, keys or reuse of their other parts):",
            );
        }
        let title = bundle
            .title(&edge.from)
            .unwrap_or_else(|_| edge.from.clone());
        let indent = "  ".repeat(level - 1);
        out.push_str(&format!(
            "\n{indent}{title} ({}){} --{}--> {id}{}",
            edge.from,
            role_note(bundle, &edge.from),
            edge.relation,
            fragments_suffix(edge)
        ));
        // A key: the file to edit to change it.
        if let Some(file) = bundle
            .node(&edge.from)
            .filter(|n| n.is_declared() && n.type_ != MISSING_TYPE)
            .and_then(|n| n.source_file.as_deref())
        {
            out.push_str(&format!(" [defined in {file}]"));
        }
        if let Some(excerpt) = excerpt_by_id.get(edge.from.as_str()) {
            out.push_str(&format!("\n{indent}  {excerpt}"));
        }
    }
    if end < total {
        out.push_str(&format!(
            "\n\n(showing {}-{end} of {total} -- pass offset={end} for the next page, \
             or narrow with relation, fragment or depth; excerpts=false shortens each entry)",
            offset + 1
        ));
    }
    Ok(out)
}

/// How a change reaches a concept [`analyze_impact`] meets, which decides
/// what depends on that concept in turn.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Reach {
    /// It links to, contains, requires or uses a key to a changed
    /// concept: its own content stays the same, so nothing that points
    /// at it depends on the change.
    Points,
    /// It reuses part of the changed concept. What reuses it in turn
    /// depends on the change only when the passage it reuses holds that
    /// part, and the extractor then records that reuse straight to the
    /// changed concept (every element pulled in keeps the file it comes
    /// from): following reuse further only adds the pages that reuse its
    /// other parts. A page that reuses the fragment of a conref warehouse
    /// holding a link gets that link itself, the same way.
    Reuses,
    /// It includes a changed sample file (`coderef`): the file's text
    /// sits in it with no trace of where it came from, so the topics
    /// that reuse it are followed, once.
    Includes,
    /// The changed concept itself, a `copy-to` copy of it, or a key that
    /// names it: whatever points at it depends on the change.
    Changed,
}

impl Reach {
    /// How a concept with an edge of `relation` to one reached `self`
    /// is reached; `None` when the change does not follow that edge.
    fn through(self, relation: &str) -> Option<Reach> {
        match (self, relation) {
            (Reach::Changed, "copy-of" | "key-target") => Some(Reach::Changed),
            (Reach::Changed, "includes-code") | (Reach::Includes, "copy-of") => {
                Some(Reach::Includes)
            }
            (Reach::Changed | Reach::Includes, "generated-from") => Some(Reach::Reuses),
            (Reach::Changed, _) => Some(Reach::Points),
            (Reach::Includes | Reach::Reuses | Reach::Points, _) => None,
        }
    }
}

/// [`analyze_impact`]'s breadth-first walk, level by level, so the
/// closest dependents come first. Each hit: (level, the edge, the id it
/// points at). `every_relation` follows every edge from every concept,
/// as if any change spread through links and maps too.
///
/// A concept with several edges to the ones a level starts from keeps the
/// strongest (a topic that links to a changed one *and* reuses its
/// content is a reuser), and each level is sorted by relation and id: the
/// order of the edges in `graph.json` or `graph.db` decides nothing.
fn impact_hits<'a>(
    bundle: &'a BundleReader,
    topic_id: &str,
    depth: usize,
    relation: Option<&str>,
    fragment: Option<&str>,
    every_relation: bool,
) -> Vec<(usize, &'a crate::bundle::GraphEdge, String)> {
    let mut visited = std::collections::HashSet::new();
    let mut frontier = vec![(topic_id.to_string(), Reach::Changed)];
    let mut hits = Vec::new();
    visited.insert(topic_id.to_string());
    for level in 1..=depth {
        let mut best: std::collections::HashMap<&str, (Reach, &crate::bundle::GraphEdge, &str)> =
            Default::default();
        for (id, reach) in &frontier {
            for edge in bundle.edges_to(id, relation) {
                // The map that defines a key is where the key is written
                // (shown on the key's line), not a dependent of it. A key
                // naming the topic is followed: its users depend on the
                // topic even without a link to it (abbreviated-form,
                // keyword or term through a glossary key); the ones that
                // also link to it are already counted, once.
                if edge.relation == "defines-key" || visited.contains(&edge.from) {
                    continue;
                }
                // `fragment` only narrows the direct dependents: the
                // fragment ids belong to `topicId`'s own content.
                if level == 1
                    && let Some(fragment) = fragment
                    && !(edge.relation == "generated-from"
                        && edge.fragments.iter().any(|f| f == fragment))
                {
                    continue;
                }
                let Some(then) = reach
                    .through(&edge.relation)
                    .or(every_relation.then_some(Reach::Points))
                else {
                    continue;
                };
                let candidate = (then, edge, id.as_str());
                let rank = |(reach, edge, id): &(Reach, &crate::bundle::GraphEdge, &str)| {
                    (
                        std::cmp::Reverse(*reach),
                        edge.relation.clone(),
                        id.to_string(),
                    )
                };
                best.entry(edge.from.as_str())
                    .and_modify(|kept| {
                        if rank(&candidate) < rank(kept) {
                            *kept = candidate;
                        }
                    })
                    .or_insert(candidate);
            }
        }
        if best.is_empty() {
            break;
        }
        let mut found: Vec<(String, Reach, &crate::bundle::GraphEdge, String)> = best
            .into_iter()
            .map(|(from, (then, edge, id))| (from.to_string(), then, edge, id.to_string()))
            .collect();
        found.sort_by(|a, b| (&a.2.relation, &a.0).cmp(&(&b.2.relation, &b.0)));
        frontier = Vec::with_capacity(found.len());
        for (from, then, edge, id) in found {
            visited.insert(from.clone());
            hits.push((level, edge, id));
            frontier.push((from, then));
        }
    }
    hits
}

/// Default page size of [`analyze_impact`]: 40 entries with their
/// excerpts stay far below what an MCP client displays inline.
const IMPACT_PAGE: usize = 40;

/// The first `max_chars` characters of `text` (newlines flattened to
/// spaces, since a report line should stay one line), with a trailing
/// `…` when truncated. Char-counted, not byte-counted, so it never
/// panics on a multi-byte UTF-8 boundary.
fn excerpt(text: &str, max_chars: usize) -> String {
    let flattened = text.replace('\n', " ");
    let mut result: String = flattened.chars().take(max_chars).collect();
    if flattened.chars().count() > max_chars {
        result.push('…');
    }
    result
}

/// `topicId`, not `id` -- every other tool in this set that takes a
/// concept id (`find_related_topics`, `explain_task`,
/// `trace_dependencies`, `search_content`'s optional narrowing param,
/// `analyze_impact`) names it `topicId`; this one used to be the lone
/// `id` holdout. Found live, not by inspection: a real Claude Code
/// session driving this server over MCP called `generate_summary` with
/// `topicId` first (matching the rest of the tool set, the same way any
/// agent would reasonably infer this tool's shape from the others) and
/// got `missing required argument \`id\`` twice before it happened to
/// try `id` -- a real, reproducible usability bug caught by watching
/// live tool use, not something a hand-written test with the "correct"
/// parameter name baked in would ever catch.
fn generate_summary(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let id = arg_str(arguments, "topicId")?;
    let (frontmatter, _) = bundle.read_concept(id)?;
    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(id);
    let description = frontmatter
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("(no description in frontmatter)");
    Ok(format!("{title}: {description}"))
}

fn validate_bundle(bundle_root: &Path) -> Result<String> {
    let report = okf_validator::validate_bundle(&bundle_root.join("okf"))?;
    if report.issues.is_empty() {
        return Ok("bundle OK: no issues".to_string());
    }
    Ok(report
        .issues
        .iter()
        .map(|i| format!("{:?} {}: {}", i.severity, i.file, i.message))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Resolves `topicId` to its source `.dita` file (via the `resource`
/// frontmatter field `okf.rs`'s `render_concept` writes for every
/// concept -- the same field `explain_task`/`generate_summary` leave
/// alone but this tool specifically needs) and runs it through the
/// vendored DitaCraft LSP (`crate::live`), live, not from the bundle.
fn validate_live(
    bundle: &BundleReader,
    arguments: &Value,
    live_config: &LiveValidationConfig,
) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?;
    let (frontmatter, _) = bundle.read_concept(topic_id)?;
    let resource = frontmatter
        .get("resource")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{topic_id} has no `resource` in its frontmatter"))?;
    // A copy DITA-OT writes under another name (copy-to, branch
    // filtering) has no file of its own in the source tree: its OKF
    // `sources` entry names the file the author edits, the one to check.
    let authored = frontmatter
        .get("sources")
        .and_then(|v| v.as_sequence())
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("resource"))
        .and_then(|v| v.as_str());
    let (checked, field) = match authored {
        Some(file) => (file, "sources"),
        None => (resource, "resource"),
    };
    let copy_note = if authored.is_some() {
        format!("; {resource} is a copy DITA-OT writes from it")
    } else {
        String::new()
    };

    let source_root = live_config.source_root.as_ref().ok_or_else(|| {
        anyhow!(
            "validate_live needs the original DITA source tree -- pass --source-root, \
             set DITA2GRAPH_SOURCE_ROOT, or add a [dita] source_root to mcp-server.toml"
        )
    })?;
    // A table-of-contents heading's resource is `<map>#<heading id>`:
    // the file to validate is the map that declares it.
    let file_path = source_root.join(checked.split('#').next().unwrap_or(checked));
    if !file_path.is_file() {
        return Err(anyhow!(
            "resolved source file {} (from {topic_id}'s `{field}: {checked}`) does not \
             exist -- is --source-root pointing at the right DITA project?",
            file_path.display()
        ));
    }

    let diagnostics = live::validate_file(live_config, source_root, &file_path)?;
    if diagnostics.is_empty() {
        return Ok(format!(
            "No diagnostics -- {checked} is valid (live DitaCraft LSP check{copy_note})."
        ));
    }
    let mut lines = vec![format!(
        "{} diagnostic{} for {topic_id} ({checked}{copy_note}), live from DitaCraft's LSP validation pipeline:",
        diagnostics.len(),
        if diagnostics.len() == 1 { "" } else { "s" },
    )];
    for d in &diagnostics {
        let code = d
            .code
            .as_ref()
            .map(|c| format!(" {}", live::code_str(c)))
            .unwrap_or_default();
        let source = d
            .source
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        lines.push(format!(
            "- [{}]{code}{source} line {}: {}",
            live::severity_label(d.severity),
            d.range.start.line + 1,
            d.message,
        ));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_metadata_match_starts_a_word() {
        assert!(starts_a_word("git client", "git"));
        assert!(starts_a_word("--plugins", "plugin"));
        assert!(starts_a_word("error: java.io", "java"));
        assert!(!starts_a_word("digital signature", "git"));
        assert!(!starts_a_word("anything", ""));
    }

    #[test]
    fn metadata_matches_names_what_matched_and_counts_the_rest() {
        let metadata = dita2graph_core::Metadata {
            keywords: vec!["Setup".into(), "digital".into()],
            index_terms: vec![
                vec!["installing".into(), "setup wizard".into()],
                vec!["setup".into()],
                vec!["setup".into(), "on Linux".into()],
                vec!["setup".into(), "on Windows".into()],
            ],
            ..Default::default()
        };
        assert_eq!(
            metadata_matches(&metadata, "setup", &[]),
            "keyword \"Setup\", index \"installing > setup wizard\", index \"setup\", +2 more"
        );
        assert_eq!(metadata_matches(&metadata, "git", &[]), "");
        // Every word, the terms of one entry together, in any order.
        let words = query_words("linux setup");
        assert_eq!(
            metadata_matches(&metadata, "linux setup", &words),
            "index \"setup > on Linux\""
        );
    }

    #[test]
    fn a_query_has_words_only_when_it_has_several() {
        assert_eq!(query_words("rename id"), ["id", "rename"]);
        assert_eq!(query_words("how to rename an ID?"), ["id", "rename"]);
        assert_eq!(query_words("ID/IDREF"), ["id", "idref"]);
        assert!(query_words("plug-ins").is_empty(), "one word");
        assert_eq!(query_words("install plug-ins."), ["install", "plug-ins"]);
        assert_eq!(query_words("search and search"), ["search"]);
        // Nothing but stop words: they are the words.
        assert_eq!(query_words("how to"), ["how", "to"]);
        assert!(query_words("rename").is_empty());
        assert!(query_words("  ").is_empty());
    }

    #[test]
    fn a_long_index_shows_its_first_entries_and_counts_the_rest() {
        let metadata = dita2graph_core::Metadata {
            index_terms: (1..=27).map(|i| vec![format!("term {i}")]).collect(),
            ..Default::default()
        };
        let lines = metadata_lines(&metadata);
        assert!(lines.contains("term 20 (+7 more)"), "{lines}");
        assert!(!lines.contains("term 21"), "{lines}");
    }

    #[test]
    fn relevance_score_weighs_a_title_match_more_than_a_body_mention() {
        let title_hit = relevance_score(&["install"], "installing product", "");
        let body_hit = relevance_score(&["install"], "configuration overview", "install install");
        assert!(
            title_hit > body_hit,
            "a single title match ({title_hit}) should outweigh two body mentions ({body_hit})"
        );
    }

    #[test]
    fn relevance_score_sums_across_multiple_terms() {
        let both_terms = relevance_score(&["encryption", "keys"], "", "encryption keys");
        let one_term = relevance_score(&["encryption", "keys"], "", "encryption key");
        assert!(
            both_terms > one_term,
            "matching both query terms ({both_terms}) should outscore matching only one ({one_term})"
        );
    }

    #[test]
    fn relevance_score_counts_repeated_body_occurrences() {
        let twice = relevance_score(&["install"], "", "install install");
        let once = relevance_score(&["install"], "", "install");
        assert_eq!(twice, once * 2);
    }

    #[test]
    fn relevance_score_is_zero_for_no_match() {
        assert_eq!(relevance_score(&["nowhere"], "some title", "some text"), 0);
    }

    #[test]
    fn plain_text_drops_the_reuse_markers_and_keeps_where_each_passage_comes_from() {
        let plain = PlainText::parse(
            "Run  dita install. [reused from a.dita#outer] Outer [reused from b.dita#inner] inner \
             text [end reused] after. [end reused] Done.",
        );
        assert_eq!(
            plain.text,
            "Run dita install. Outer inner text after. Done."
        );
        let at = |s: &str| plain.text.find(s).unwrap();
        let inner = at("inner text");
        assert_eq!(plain.reused_source(inner, inner + 5), Some("b.dita#inner"));
        let outer = at("after");
        assert_eq!(plain.reused_source(outer, outer + 5), Some("a.dita#outer"));
        let own = at("Done");
        assert_eq!(plain.reused_source(own, own + 4), None);
        // A passage left open runs to the end of the text.
        let open = PlainText::parse("Intro [reused from c.dita#x] tail");
        assert_eq!(open.text, "Intro tail");
        assert_eq!(open.reused_source(6, 10), Some("c.dita#x"));
    }

    #[test]
    fn plain_text_finds_the_notes_for_the_authors() {
        let plain = PlainText::parse(
            "Run it. [draft-comment] Check this. [end draft-comment] Then [required-cleanup] old \
             [end required-cleanup] done. [draft-comment] left open",
        );
        let at = |s: &str| plain.text.find(s).unwrap();
        assert_eq!(
            plain.note_kind(at("Check"), at("Check") + 5),
            Some("draft-comment")
        );
        assert_eq!(
            plain.note_kind(at("old"), at("old") + 3),
            Some("required-cleanup")
        );
        assert_eq!(plain.note_kind(at("Then"), at("Then") + 4), None);
        assert_eq!(
            plain.note_kind(at("left"), at("left") + 4),
            Some("draft-comment")
        );
        let end = at("[end draft-comment]");
        assert!(plain.in_note_end(end + 5, end + 18));
        assert!(!plain.in_note_end(at("[draft-comment]"), at("[draft-comment]") + 15));
    }

    #[test]
    fn edit_distance_stops_past_the_maximum() {
        assert_eq!(edit_distance("pluginsdir", "plugindirs", 2), Some(2));
        assert_eq!(edit_distance("install", "instal", 1), Some(1));
        assert_eq!(edit_distance("plugins", "plugindirs", 2), None);
        assert_eq!(edit_distance("same", "same", 1), Some(0));
    }

    #[test]
    fn near_spellings_keep_misspellings_and_leave_other_words_out() {
        let words: std::collections::HashMap<String, (usize, std::collections::HashSet<&str>)> = [
            ("pluginsdir", 3),
            ("plugindir", 1),
            ("conference", 1),
            ("uninstall", 14),
            ("re-install", 2),
            ("instal", 1),
        ]
        .into_iter()
        .map(|(w, n)| (w.to_string(), (n, std::collections::HashSet::from(["t"]))))
        .collect();
        let names = |needle: &str| -> Vec<String> {
            near_spellings(needle, &words)
                .into_iter()
                .map(|(w, ..)| w)
                .collect()
        };
        assert_eq!(names("plugindirs"), ["pluginsdir"]);
        assert!(names("confluence").is_empty());
        // Compared without its leading dashes: `uninstall` and `re-install`
        // contain `install`, `instal` is contained in it.
        assert!(names("--install").is_empty());
    }

    #[test]
    fn context_cuts_on_character_boundaries() {
        let text = format!("{}pluginsdir{}", "é".repeat(60), "è".repeat(60));
        let start = text.find("pluginsdir").unwrap();
        let line = context(&text, start, start + "pluginsdir".len());
        assert!(line.starts_with('…') && line.ends_with('…'), "{line}");
        assert!(line.contains("pluginsdir"), "{line}");
    }

    #[test]
    fn excerpt_passes_short_text_through_unchanged() {
        assert_eq!(excerpt("short text", 140), "short text");
    }

    #[test]
    fn excerpt_truncates_long_text_with_an_ellipsis() {
        let long = "a".repeat(200);
        let result = excerpt(&long, 140);
        assert_eq!(result.chars().count(), 141); // 140 chars + the ellipsis
        assert!(result.ends_with('…'));
    }

    #[test]
    fn excerpt_flattens_newlines_to_spaces() {
        assert_eq!(excerpt("line one\nline two", 140), "line one line two");
    }

    #[test]
    fn excerpt_is_safe_on_multi_byte_utf8_near_the_truncation_boundary() {
        // Each "é" is 2 bytes in UTF-8; a byte-counted truncation at an
        // odd offset would panic or produce invalid UTF-8. Char-counted
        // truncation must not.
        let text = "é".repeat(150);
        let result = excerpt(&text, 140);
        assert_eq!(result.chars().count(), 141);
    }
}
