//! Tool definitions and dispatch (§5.2, §5.5): `tools/list`'s result and
//! the implementation behind `tools/call`, adapted from the `okf-mcp`
//! pattern in `docs/plugin-specification.md` §5.5 — swap `okf-query`'s
//! generic graph/search calls for the DITA-relation-aware ones here.

use crate::bundle::{BundleCache, BundleReader};
use crate::live::{self, LiveValidationConfig};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::Path;

/// The tool list; `query_graph` only when the server runs with `--sparql`.
pub fn list(sparql: bool) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "search_topics",
            "description": "Free-text search over the titles and ids of topics and maps, and of the keys (id key:<name>), external URLs (id = the URL) and sample files (id file:<path>) they declare. Titles and ids that contain the query as a phrase come first. For a query of several words, those that have every word (each at the start of a word, in any order, stop words such as 'how', 'to', 'the' left out) follow under 'Title or id with every word of': 'rename id' finds 'Search and Rename Actions for IDs'. Then the authors' keywords and index entries: a topic whose title does not match but whose keyword or index entry has a word starting with the query, or every word of it, is listed under 'Indexed under or tagged with', with the entries that matched. Use this first to find an id before calling the other tools. Every tool marks a topic the map uses only as a resource (a reuse warehouse, a key target) with [resource-only: not a published page]: its content may appear elsewhere, but it is not a page of the publication.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Search text" } },
                "required": ["query"],
            },
        }),
        json!({
            "name": "read_topic",
            "description": "The full text of one topic, as a reader of the published page sees it: summary and body, with every fragment it reuses by conref/conkeyref kept in place between [reused from <file>#<fragment>] and [end reused] markers, then the list of reused fragments by source topic. Use this whenever the answer is in what a topic says -- the other tools only return short excerpts.",
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
            "description": "Full-text search over rag/chunks.jsonl's topic body/summary text (§13.1) -- unlike search_topics (title/id only), this searches actual content, and each hit includes a text excerpt. Ranked by keyword frequency, plus cosine similarity against rag/embeddings.jsonl when this server was started with an embedding model configured -- a strong semantic match can surface even with no shared words, not just literal term overlap. Returns at most the top 15 matches by relevance; pass topicId (optionally with relation/depth) to narrow the search to topics reachable from that id via the graph first: the hybrid pattern in §13.1 -- cheap, deterministic graph narrowing, then content search only within that smaller set, instead of the whole bundle. Excerpts are 200 characters: call read_topic on a hit to read it in full.",
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
            "name": "analyze_impact",
            "description": "Find the concepts that changing a topic id may affect -- or a key (key:<name>), an external URL or a sample file (file:<path>): everything that points at it directly (containing maps, referrers and prerequisites, topics using a key, linking to a URL directly or through a key, or including a file), and the topics whose content reuses it (a topic reusing a passage that holds some of it reuses it too, directly). A concept that only links to, contains or uses a key to one of those, or reuses another part of one, keeps its own content: the first line counts these apart, and cascade=true lists them after the others. Closest first, with a text excerpt under each affected concept. The first line gives the total count, by relation and by level, so 'how many' needs no paging. Results are paginated (40 entries by default: pass offset for the next page). To find the topics that reuse one fragment of a conref source, pass fragment=<fragment id> with depth=1.",
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
        return validate_live(bundle, arguments, &live_config);
    }
    if name == "query_graph" && !cache.sparql_enabled() {
        return Err(anyhow!(
            "query_graph is not enabled on this server (start dita2graph-mcp with --sparql)"
        ));
    }
    let embedder = cache.embedder();
    let bundle = cache.get()?;
    match name {
        "query_graph" => query_graph(bundle, arguments),
        "search_topics" => search_topics(bundle, arguments),
        "search_content" => search_content(bundle, arguments, embedder.as_deref()),
        "read_topic" => read_topic(bundle, arguments),
        "find_related_topics" => find_related_topics(bundle, arguments),
        "explain_task" => explain_task(bundle, arguments),
        "trace_dependencies" => trace_dependencies(bundle, arguments),
        "analyze_impact" => analyze_impact(bundle, arguments),
        "generate_summary" => generate_summary(bundle, arguments),
        other => Err(anyhow!("unknown tool: {other}")),
    }
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
    hits.sort();
    by_words.sort();
    by_metadata.sort();
    if hits.is_empty() && by_words.is_empty() && by_metadata.is_empty() {
        return Ok(format!("no topics matched `{query}`"));
    }
    for (heading, mut group) in [
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

    let reused = bundle.edges_from(topic_id, Some("generated-from"));
    if !reused.is_empty() {
        out.push_str("\nReused content (conref/conkeyref), by source topic:\n");
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

/// ` [resource-only: not a published page]` after a topic the map uses
/// only as a resource -- a reuse warehouse, a key target -- empty for any
/// other node. Without it, an agent listing the topics that link to a URL
/// counted the warehouse holding a reused link as one more published page
/// (dita2graph-demo/bench, campaign 4 replay, question r-o8).
fn role_note(bundle: &BundleReader, id: &str) -> &'static str {
    match bundle.node(id).and_then(|n| n.processing_role.as_deref()) {
        Some("resource-only") => " [resource-only: not a published page]",
        _ => "",
    }
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
        if let Some(file) = &node.source_file {
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
            .filter(|n| n.is_declared())
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
