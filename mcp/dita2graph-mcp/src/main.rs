//! Minimal MCP server exposing a DITA2Graph OKF bundle (§4) to AI agents,
//! following the `okf-mcp` JSON-RPC-over-stdio pattern documented in
//! `docs/plugin-specification.md` §5.5 (adapted from `jyjeanne/okf-rs`,
//! MIT/Apache-2.0) — this file mirrors that crate's `main.rs` almost
//! verbatim, with `tools::list`/`tools::call` swapped for the DITA-
//! specific tool set in §5.2.
//!
//! Speaks JSON-RPC 2.0 over stdio, one message per line: requests read
//! from stdin, responses written to stdout. `stdout` is reserved for
//! protocol messages only — all diagnostics go to stderr — since a stray
//! print would corrupt the stream for whatever's reading it.

mod bundle;
mod tools;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bundle_root = resolve_bundle_root(&args)?;
    // One cache for the whole process lifetime, not reopened per
    // request (bundle::BundleCache's own docs) -- a real agent session
    // against a real, sizeable bundle issues many tool calls, not one.
    let mut cache = bundle::BundleCache::new(bundle_root);

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("dita2graph-mcp: failed to read request line: {e}");
                continue;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("dita2graph-mcp: failed to parse request: {e}");
                continue;
            }
        };
        if let Some(response) = handle_message(&request, &mut cache) {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct McpServerConfig {
    graph: GraphConfig,
}

#[derive(Deserialize)]
struct GraphConfig {
    okf: String,
}

/// The bundle root to serve, from either a bare positional path (the
/// existing invocation) or `--config <path>` pointing at an
/// `mcp-server.toml` `dita2graph-core build --mcp true` wrote (§2.3,
/// §5.4) -- reads its `graph.okf` value, resolves it relative to the
/// config file's own directory, and takes *that* path's parent to get
/// the bundle root `BundleReader` expects (`okf/` and `mcp/` are
/// siblings under the bundle root, §2.4). Falls back to `.` when
/// nothing is given, same as before `--config` existed.
fn resolve_bundle_root(args: &[String]) -> Result<PathBuf> {
    if args.first().map(String::as_str) == Some("--config") {
        let config_path = args
            .get(1)
            .ok_or_else(|| anyhow!("--config requires a path argument"))?;
        return bundle_root_from_config(Path::new(config_path));
    }
    Ok(args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".")))
}

fn bundle_root_from_config(config_path: &Path) -> Result<PathBuf> {
    let raw = fs_read_to_string(config_path)?;
    let config: McpServerConfig =
        toml::from_str(&raw).with_context(|| format!("parsing {}", config_path.display()))?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let okf_path = config_dir.join(&config.graph.okf);
    let okf_path = okf_path.canonicalize().with_context(|| {
        format!(
            "resolving graph.okf ({}) from {}",
            config.graph.okf,
            config_path.display()
        )
    })?;
    okf_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("{} has no parent directory", okf_path.display()))
}

fn fs_read_to_string(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Dispatches one JSON-RPC message, returning the response to write (or
/// `None` for notifications, which never get one).
fn handle_message(request: &Value, cache: &mut bundle::BundleCache) -> Option<Value> {
    let method = request.get("method")?.as_str()?;
    let id = request.get("id").cloned();

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "dita2graph-mcp", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match tools::call(name, &arguments, cache) {
                Ok(text) => Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false,
                })),
                Err(e) => Ok(json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true,
                })),
            }
        }
        "notifications/initialized"
        | "notifications/cancelled"
        | "notifications/roots/list_changed" => {
            return None;
        }
        other => Err(format!("method not found: {other}")),
    };

    let id = id?;
    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(message) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": message },
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dita2graph_core::{
        Link, NormalizedMap, NormalizedNode, NormalizedTopic, Relation, TopicType, write_bundle,
        write_mcp_config, write_rag_index,
    };

    #[test]
    fn resolve_bundle_root_uses_a_real_config_file() {
        let dir = sample_bundle_root();
        write_mcp_config(dir.path()).unwrap();
        let config_path = dir
            .path()
            .join("mcp/mcp-server.toml")
            .to_string_lossy()
            .to_string();

        let resolved = resolve_bundle_root(&["--config".to_string(), config_path]).unwrap();
        assert_eq!(
            resolved.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn resolve_bundle_root_falls_back_to_a_positional_path() {
        let resolved = resolve_bundle_root(&["some/bundle/dir".to_string()]).unwrap();
        assert_eq!(resolved, PathBuf::from("some/bundle/dir"));
    }

    #[test]
    fn resolve_bundle_root_defaults_to_dot_with_no_args() {
        let resolved = resolve_bundle_root(&[]).unwrap();
        assert_eq!(resolved, PathBuf::from("."));
    }

    #[test]
    fn resolve_bundle_root_errors_when_config_flag_has_no_path() {
        assert!(resolve_bundle_root(&["--config".to_string()]).is_err());
    }

    fn sample_bundle_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let nodes = vec![
            NormalizedNode::Map(NormalizedMap {
                id: "user-guide".into(),
                title: "User Guide".into(),
                source_file: "user-guide.ditamap".into(),
                links: vec![Link {
                    relation: Relation::Contains,
                    target: "installing-product".into(),
                }],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "installing-product".into(),
                topic_type: TopicType::Task,
                title: "Installing Product".into(),
                shortdesc: Some("Steps to install the product.".into()),
                body: None,
                audience: vec!["admin".into()],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/installing-product.dita".into(),
                links: vec![Link {
                    relation: Relation::Requires,
                    target: "configuration".into(),
                }],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "configuration".into(),
                topic_type: TopicType::Concept,
                title: "Configuration Overview".into(),
                shortdesc: None,
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/configuration.dita".into(),
                links: vec![],
            }),
        ];
        write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
        dir
    }

    /// A bundle shaped to exercise `search_content`'s graph-narrowing:
    /// `installing-product` (`requires`) `configuration`, both with
    /// "encryption" somewhere in their text, plus a third topic with the
    /// same word that's *not* reachable from `installing-product` --
    /// scoped search should find the first two and not the third.
    fn content_search_bundle_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let nodes = vec![
            NormalizedNode::Map(NormalizedMap {
                id: "user-guide".into(),
                title: "User Guide".into(),
                source_file: "user-guide.ditamap".into(),
                links: vec![
                    Link {
                        relation: Relation::Contains,
                        target: "installing-product".into(),
                    },
                    Link {
                        relation: Relation::Contains,
                        target: "unrelated-topic".into(),
                    },
                ],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "installing-product".into(),
                topic_type: TopicType::Task,
                title: "Installing Product".into(),
                shortdesc: Some("Steps to install the product.".into()),
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/installing-product.dita".into(),
                links: vec![Link {
                    relation: Relation::Requires,
                    target: "configuration".into(),
                }],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "configuration".into(),
                topic_type: TopicType::Concept,
                title: "Configuration Overview".into(),
                shortdesc: None,
                body: Some("Set the encryption key before starting.".into()),
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/configuration.dita".into(),
                links: vec![],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "unrelated-topic".into(),
                topic_type: TopicType::Concept,
                title: "Unrelated Topic".into(),
                shortdesc: None,
                body: Some("Encryption keys must be rotated regularly.".into()),
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/unrelated-topic.dita".into(),
                links: vec![],
            }),
        ];
        write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
        write_rag_index(&nodes, dir.path(), chrono::Utc::now()).unwrap();
        dir
    }

    #[test]
    fn initialize_reports_capabilities() {
        let response = handle_message(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            &mut bundle::BundleCache::new(PathBuf::from(".")),
        )
        .unwrap();
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn notification_gets_no_response() {
        let response = handle_message(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            &mut bundle::BundleCache::new(PathBuf::from(".")),
        );
        assert!(response.is_none());
    }

    #[test]
    fn tools_list_includes_the_dita_specific_tools() {
        let response = handle_message(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            &mut bundle::BundleCache::new(PathBuf::from(".")),
        )
        .unwrap();
        let names: Vec<&str> = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"search_topics"));
        assert!(names.contains(&"search_content"));
        assert!(names.contains(&"find_related_topics"));
        assert!(names.contains(&"analyze_impact"));
        assert!(names.contains(&"validate_bundle"));
    }

    #[test]
    fn search_topics_finds_the_installing_task() {
        let dir = sample_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "search_topics", "arguments": { "query": "installing" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Installing Product"));
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn find_related_topics_follows_requires_edge() {
        let dir = sample_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "find_related_topics", "arguments": { "topicId": "installing-product", "relation": "requires" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Configuration Overview"));
    }

    #[test]
    fn search_content_finds_a_match_in_body_text_not_just_the_title() {
        let dir = content_search_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "search_content", "arguments": { "query": "encryption" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        // Unscoped: both topics containing "encryption" should be found,
        // even though the query never matches either title.
        assert!(text.contains("Configuration Overview"), "{text}");
        assert!(text.contains("Unrelated Topic"), "{text}");
        assert_eq!(response["result"]["isError"], false);
    }

    /// Found live: a real Claude Code session asking a content question
    /// got titles/scores back from search_content but no way to see
    /// *what actually matched* without a second round trip -- and no
    /// other tool filled that gap either. Each hit should carry a short
    /// excerpt of the matched text now, not just its title/id/score.
    #[test]
    fn search_content_includes_a_text_excerpt_for_each_match() {
        let dir = content_search_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "search_content", "arguments": { "query": "encryption" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("Set the encryption key before starting."),
            "{text}"
        );
        assert!(
            text.contains("Encryption keys must be rotated regularly."),
            "{text}"
        );
    }

    /// Same gap, found in the same live session: explain_task fetched a
    /// topic's body via read_concept and threw it away (`let
    /// (frontmatter, _body) = ...`), leaving no tool at all that could
    /// answer "what does this topic actually say" -- title and a
    /// one-sentence shortdesc (often absent) was the closest anything
    /// got.
    #[test]
    fn explain_task_includes_a_body_excerpt() {
        let dir = content_search_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "explain_task", "arguments": { "topicId": "configuration" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("Set the encryption key before starting."),
            "{text}"
        );
    }

    #[test]
    fn search_content_ranks_by_multi_term_frequency_not_alphabetically() {
        let dir = content_search_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "search_content", "arguments": { "query": "encryption keys" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        // "Encryption keys must be rotated regularly." (unrelated-topic)
        // matches both terms; "Set the encryption key before starting."
        // (configuration) only matches "encryption" ("key" != "keys").
        // Alphabetically "Configuration Overview" would sort first --
        // ranked by score, "Unrelated Topic" (the better match) must
        // come first instead.
        let unrelated_pos = text.find("Unrelated Topic").expect(text);
        let configuration_pos = text.find("Configuration Overview").expect(text);
        assert!(
            unrelated_pos < configuration_pos,
            "expected the higher-scoring match first:\n{text}"
        );
    }

    #[test]
    fn search_content_scoped_to_a_topic_id_narrows_via_the_graph_first() {
        let dir = content_search_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "search_content", "arguments": { "query": "encryption", "topicId": "installing-product" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        // configuration is reachable from installing-product (requires);
        // unrelated-topic is not, even though it also matches "encryption".
        assert!(text.contains("Configuration Overview"), "{text}");
        assert!(!text.contains("Unrelated Topic"), "{text}");
    }

    /// Found live: on a real, sizeable corpus, an unscoped or broad
    /// query matching dozens of topics -- each now carrying its own
    /// excerpt -- produced 50+ KB of output, past what a real MCP
    /// client renders inline. 20 matching topics here, well past the
    /// 15-result cap, proves both halves: only the top 15 (by score)
    /// come back, and the response says so rather than silently
    /// dropping the rest.
    #[test]
    fn search_content_caps_results_and_notes_the_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let nodes: Vec<NormalizedNode> = (0..20)
            .map(|i| {
                NormalizedNode::Topic(NormalizedTopic {
                    id: format!("topic-{i}"),
                    topic_type: TopicType::Concept,
                    title: format!("Topic {i}"),
                    shortdesc: None,
                    body: Some("Mentions widgets in its body.".into()),
                    audience: vec![],
                    product: vec![],
                    keys: vec![],
                    uicontrols: vec![],
                    cmd_uicontrols: vec![],
                    source_file: format!("topics/topic-{i}.dita"),
                    links: vec![],
                })
            })
            .collect();
        write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
        write_rag_index(&nodes, dir.path(), chrono::Utc::now()).unwrap();

        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "search_content", "arguments": { "query": "widgets" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let result_count = text.matches("Mentions widgets in its body.").count();
        assert_eq!(result_count, 15, "{text}");
        assert!(
            text.contains("showing top 15 of 20 matches"),
            "expected a truncation note: {text}"
        );
    }

    #[test]
    fn search_content_reports_no_rag_index_when_bundle_predates_rag() {
        let dir = sample_bundle_root(); // built without write_rag_index
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "search_content", "arguments": { "query": "anything" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("no rag/chunks.jsonl found"), "{text}");
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn analyze_impact_finds_transitive_dependents() {
        let dir = sample_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "analyze_impact", "arguments": { "topicId": "configuration" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        // installing-product requires configuration (1 hop); user-guide
        // contains installing-product (2 hops) -- both should show up as
        // affected, not just the direct dependent.
        assert!(text.contains("installing-product"), "{text}");
        assert!(text.contains("user-guide"), "{text}");
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn analyze_impact_includes_a_content_excerpt_when_rag_has_one() {
        let dir = content_search_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "analyze_impact", "arguments": { "topicId": "configuration" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        // installing-product requires configuration, and has rag/ text
        // (its shortdesc) -- the excerpt should appear right under it.
        assert!(text.contains("installing-product"), "{text}");
        assert!(text.contains("Steps to install the product."), "{text}");
        // user-guide (a map) is also an affected concept (2 hops, via
        // "contains installing-product") but maps aren't chunked into
        // rag/ (§13.1), so it gets no excerpt line -- just confirm the
        // overall report still lists it without crashing on the lookup.
        assert!(text.contains("user-guide"), "{text}");
    }

    #[test]
    fn analyze_impact_reports_nothing_for_a_leaf_with_no_dependents() {
        let dir = sample_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "analyze_impact", "arguments": { "topicId": "user-guide" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("nothing depends on"), "{text}");
    }

    #[test]
    fn generate_summary_returns_title_and_description_for_a_topic_id() {
        let dir = sample_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "generate_summary", "arguments": { "topicId": "installing-product" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, "Installing Product: Steps to install the product.");
        assert_eq!(response["result"]["isError"], false);
    }

    /// `topicId`, matching every other tool in the set -- not `id`
    /// (found live: a real Claude Code session calling this tool with
    /// `topicId` first, the same way any agent would reasonably infer
    /// this tool's shape from the rest of the set, hit exactly this
    /// error twice before it happened to try `id`). The error message
    /// itself needs to name the parameter this tool actually expects,
    /// or a caller that DOES get this wrong has no way to self-correct
    /// from the error alone.
    #[test]
    fn generate_summary_reports_a_tool_level_error_naming_the_correct_parameter() {
        let dir = sample_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "generate_summary", "arguments": { "id": "installing-product" } }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("topicId"), "{text}");
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn unknown_tool_reports_a_tool_level_error_not_a_protocol_error() {
        let dir = sample_bundle_root();
        let response = handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "does_not_exist", "arguments": {} }
            }),
            &mut bundle::BundleCache::new(dir.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], true);
    }
}
