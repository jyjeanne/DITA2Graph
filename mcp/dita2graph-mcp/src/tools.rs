//! Tool definitions and dispatch (§5.2, §5.5): `tools/list`'s result and
//! the implementation behind `tools/call`, adapted from the `okf-mcp`
//! pattern in `docs/plugin-specification.md` §5.5 — swap `okf-query`'s
//! generic graph/search calls for the DITA-relation-aware ones here.

use crate::bundle::{BundleCache, BundleReader};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::Path;

pub fn list() -> Vec<Value> {
    vec![
        json!({
            "name": "search_topics",
            "description": "Free-text search over topic/map titles and ids. Use this first to find a topic's id before calling the other tools.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Search text" } },
                "required": ["query"],
            },
        }),
        json!({
            "name": "find_related_topics",
            "description": "List concepts related to the given topic id, optionally filtered to one relation (contains/references/related-to/applies-to/requires/generated-from, §4.3).",
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
            "description": "Title, description, a body excerpt, and requires/contains/applies-to relations for a topic id.",
            "inputSchema": {
                "type": "object",
                "properties": { "topicId": { "type": "string" } },
                "required": ["topicId"],
            },
        }),
        json!({
            "name": "trace_dependencies",
            "description": "Follow `requires` edges from a topic id up to a given depth.",
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
            "description": "Full-text search over rag/chunks.jsonl's topic body/summary text (§13.1) -- unlike search_topics (title/id only), this searches actual content. Pass topicId (optionally with relation/depth) to narrow the search to topics reachable from that id via the graph first: the hybrid pattern in §13.1 -- cheap, deterministic graph narrowing, then content search only within that smaller set, instead of the whole bundle.",
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
            "description": "Find every concept that would be affected by changing a topic id: a reverse graph traversal over all relations (dependents, containing maps, requires/keyref referrers), not just its direct links, with a text excerpt from rag/chunks.jsonl under each affected concept when one exists (§13.1).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topicId": { "type": "string" },
                    "depth": { "type": "integer", "description": "Max traversal depth (default 5)" },
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
    ]
}

/// Dispatches one `tools/call`. `validate_bundle` is deliberately
/// special-cased ahead of `cache.get()`: it's meant to check the
/// bundle's live on-disk state (`§2.5`/`§6.4`/`§10`), not go through the
/// cached `BundleReader` at all -- and unlike every other tool here, it
/// doesn't need `graph.json` to exist to do its job, so it shouldn't
/// fail just because a `dita2graph-core build` hasn't produced one yet.
pub fn call(name: &str, arguments: &Value, cache: &mut BundleCache) -> Result<String> {
    if name == "validate_bundle" {
        return validate_bundle(cache.root());
    }
    let bundle = cache.get()?;
    match name {
        "search_topics" => search_topics(bundle, arguments),
        "search_content" => search_content(bundle, arguments),
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
    let mut hits: Vec<String> = Vec::new();
    for node in bundle.all_nodes() {
        let title = bundle.title(&node.id).unwrap_or_else(|_| node.id.clone());
        if node.id.to_lowercase().contains(&query) || title.to_lowercase().contains(&query) {
            hits.push(format!("{} ({}) [{}]", title, node.type_, node.id));
        }
    }
    hits.sort();
    if hits.is_empty() {
        return Ok(format!("no topics matched `{query}`"));
    }
    Ok(hits.join("\n"))
}

/// Content search over `rag/chunks.jsonl` (§13.1), optionally scoped to
/// the topics reachable from `topicId` via a forward graph traversal --
/// the "graph narrows first, content search runs only on what's left"
/// pattern that section describes, made concrete: `search_topics` above
/// only ever matches titles/ids against `okf/`, never a topic's actual
/// prose. Results are ranked by keyword-frequency score (see
/// `relevance_score`), not returned in an arbitrary/alphabetical order
/// the way `search_topics` still is.
fn search_content(bundle: &BundleReader, arguments: &Value) -> Result<String> {
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

    let allowed = scope_topic.map(|id| forward_reachable(bundle, id, relation, depth));

    let mut scored: Vec<(i64, &str, &str, &str, Option<String>)> = Vec::new();
    for chunk in chunks.iter() {
        if let Some(allowed) = &allowed
            && !allowed.contains(&chunk.id)
        {
            continue;
        }
        let title_lower = chunk.title.to_lowercase();
        let text_lower = chunk.text.as_deref().unwrap_or_default().to_lowercase();
        let score = relevance_score(&terms, &title_lower, &text_lower);
        if score > 0 {
            scored.push((
                score,
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
    }
    // Highest score first; tie-break by id so the ordering is
    // deterministic rather than dependent on chunks.jsonl's file order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));

    if scored.is_empty() {
        let query = terms.join(" ");
        return Ok(match scope_topic {
            Some(id) => format!("no content matched `{query}` within topics reachable from `{id}`"),
            None => format!("no content matched `{query}`"),
        });
    }
    Ok(scored
        .into_iter()
        .map(|(score, id, title, topic_type, text_excerpt)| {
            let mut line = format!("{title} ({topic_type}) [{id}] (score: {score})");
            if let Some(text_excerpt) = text_excerpt {
                line.push_str(&format!("\n  {text_excerpt}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n"))
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

fn find_related_topics(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?;
    let relation = arguments.get("relation").and_then(|v| v.as_str());
    let edges = bundle.edges_from(topic_id, relation);
    if edges.is_empty() {
        return Ok(format!("no relations found for `{topic_id}`"));
    }
    Ok(edges
        .iter()
        .map(|e| {
            let title = bundle.title(&e.to).unwrap_or_else(|_| e.to.clone());
            format!("{} -> {} ({})", e.relation, title, e.to)
        })
        .collect::<Vec<_>>()
        .join("\n"))
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

    let mut out = format!("Topic: {title} ({topic_id})\n{description}\n");
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
                out.push_str(&format!("  - {target_title} ({})\n", e.to));
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
                        "{}{} requires {} ({})",
                        "  ".repeat(level - 1),
                        id,
                        title,
                        edge.to
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

/// Reverse BFS over *every* relation, not just `requires` like
/// [`trace_dependencies`] -- "if I change this topic, what breaks?"
/// needs `contains` (which maps/topics include it) and `references` too,
/// not only declared prerequisites (§13.1). Each affected concept gets a
/// text excerpt from `rag/chunks.jsonl` under it when one exists -- the
/// "content layer summarizes the impact" half of §13.1's design text,
/// implemented as a raw excerpt handed to the calling agent rather than
/// a server-generated summary (this tool doesn't call an LLM); the
/// agent's own read of the excerpts is the summary.
fn analyze_impact(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?.to_string();
    let depth = arguments.get("depth").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

    // Best-effort enrichment: a missing or malformed rag/ shouldn't break
    // the graph traversal, which is this tool's primary job.
    let chunks = bundle.rag_chunks().unwrap_or_default();
    let excerpt_by_id: std::collections::HashMap<&str, String> = chunks
        .iter()
        .filter_map(|c| c.text.as_deref().map(|t| (c.id.as_str(), excerpt(t, 140))))
        .collect();

    let mut visited = std::collections::HashSet::new();
    let mut frontier = vec![topic_id.clone()];
    let mut lines = Vec::new();
    visited.insert(topic_id.clone());

    for level in 1..=depth {
        let mut next = Vec::new();
        for id in &frontier {
            for edge in bundle.edges_to(id, None) {
                if visited.insert(edge.from.clone()) {
                    let title = bundle
                        .title(&edge.from)
                        .unwrap_or_else(|_| edge.from.clone());
                    let indent = "  ".repeat(level - 1);
                    let mut line = format!(
                        "{indent}{title} ({}) --{}--> {id}",
                        edge.from, edge.relation
                    );
                    if let Some(excerpt) = excerpt_by_id.get(edge.from.as_str()) {
                        line.push_str(&format!("\n{indent}  {excerpt}"));
                    }
                    lines.push(line);
                    next.push(edge.from.clone());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    if lines.is_empty() {
        return Ok(format!(
            "nothing depends on `{topic_id}` (no incoming edges found)"
        ));
    }
    Ok(format!(
        "changing `{topic_id}` would affect {} concept(s):\n{}",
        lines.len(),
        lines.join("\n")
    ))
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
