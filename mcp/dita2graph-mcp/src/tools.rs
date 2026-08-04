//! Tool definitions and dispatch (§5.2, §5.5): `tools/list`'s result and
//! the implementation behind `tools/call`, adapted from the `okf-mcp`
//! pattern in `docs/plugin-specification.md` §5.5 — swap `okf-query`'s
//! generic graph/search calls for the DITA-relation-aware ones here.

use crate::bundle::BundleReader;
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
            "description": "Title, description, and requires/contains relations for a topic id.",
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
            "description": "Find every concept that would be affected by changing a topic id: a reverse graph traversal over all relations (dependents, containing maps, requires/keyref referrers), not just its direct links (§13.1).",
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
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
            },
        }),
        json!({
            "name": "validate_bundle",
            "description": "Re-run okf-validator conformance checks against the bundle this server is bound to (§2.5, §6.4, §10).",
            "inputSchema": { "type": "object", "properties": {} },
        }),
    ]
}

pub fn call(name: &str, arguments: &Value, bundle_root: &Path) -> Result<String> {
    let bundle = BundleReader::open(bundle_root)?;
    match name {
        "search_topics" => search_topics(&bundle, arguments),
        "search_content" => search_content(&bundle, arguments),
        "find_related_topics" => find_related_topics(&bundle, arguments),
        "explain_task" => explain_task(&bundle, arguments),
        "trace_dependencies" => trace_dependencies(&bundle, arguments),
        "analyze_impact" => analyze_impact(&bundle, arguments),
        "generate_summary" => generate_summary(&bundle, arguments),
        "validate_bundle" => validate_bundle(bundle_root),
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
/// prose.
fn search_content(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let query = arg_str(arguments, "query")?.to_lowercase();
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

    let mut hits: Vec<String> = Vec::new();
    for chunk in &chunks {
        if let Some(allowed) = &allowed
            && !allowed.contains(&chunk.id)
        {
            continue;
        }
        let text = chunk.text.as_deref().unwrap_or_default().to_lowercase();
        if chunk.title.to_lowercase().contains(&query) || text.contains(&query) {
            hits.push(format!(
                "{} ({}) [{}]",
                chunk.title, chunk.topic_type, chunk.id
            ));
        }
    }
    hits.sort();

    if hits.is_empty() {
        return Ok(match scope_topic {
            Some(id) => format!("no content matched `{query}` within topics reachable from `{id}`"),
            None => format!("no content matched `{query}`"),
        });
    }
    Ok(hits.join("\n"))
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
/// not only declared prerequisites (§13.1, the first implemented tool
/// from that section's design direction).
fn analyze_impact(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let topic_id = arg_str(arguments, "topicId")?.to_string();
    let depth = arguments.get("depth").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

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
                    lines.push(format!(
                        "{}{} ({}) --{}--> {}",
                        "  ".repeat(level - 1),
                        title,
                        edge.from,
                        edge.relation,
                        id
                    ));
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

fn generate_summary(bundle: &BundleReader, arguments: &Value) -> Result<String> {
    let id = arg_str(arguments, "id")?;
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
