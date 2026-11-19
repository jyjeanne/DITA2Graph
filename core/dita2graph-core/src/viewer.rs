//! `<output>/graph.html`: the graph in one self-contained page a browser
//! opens from disk (`build --viewer true`, `args.dita2graph.viewer`).
//!
//! The page draws the relations by type, each in its own color, with a
//! check box to show or hide it, and colors the topics by the chapter of
//! the map they sit in or by their type. It holds its data and its script
//! (`viewer.html`, no library, no network), so it works offline and needs
//! no web server: a page opened from `file://` cannot fetch `graph.json`.
//!
//! The data is `graph.json`'s nodes and edges plus what the page needs and
//! `graph.json` leaves to the concept files: every topic's title, and its
//! chapter, the map's top-level entry it is under. No topic text: the page
//! names the topics, it does not copy their content, so it adds nothing for
//! the secret scan to check that `okf/` does not already hold.

use crate::graph_view::GraphView;
use crate::model::{NormalizedNode, Relation};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;

/// The page's file name, under the output directory.
pub const VIEWER_FILE: &str = "graph.html";

/// The `<meta name="generator">` the page carries, which tells a page this
/// build wrote from any other `graph.html` (`remove_stale_viewer`).
pub const VIEWER_MARKER: &str = "dita2graph-viewer";

const TEMPLATE: &str = include_str!("viewer.html");
const DATA_PLACEHOLDER: &str = "__DITA2GRAPH_DATA__";

#[derive(Serialize)]
struct Payload<'a> {
    title: String,
    generated: String,
    chapters: Vec<Chapter>,
    nodes: Vec<Node<'a>>,
    edges: &'a [crate::graph_view::ViewEdge],
}

#[derive(Serialize)]
struct Chapter {
    id: String,
    title: String,
}

#[derive(Serialize)]
struct Node<'a> {
    id: &'a str,
    title: &'a str,
    #[serde(rename = "type")]
    type_: &'a str,
    /// Index into `chapters`; none for the map, a topic outside the table
    /// of contents and a key, URL or file.
    #[serde(skip_serializing_if = "Option::is_none")]
    chapter: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
    /// `topics`/`maps`: the concept file is `okf/<dir>/<id>.md`. None for
    /// a key, URL or file, which has no concept file.
    #[serde(skip_serializing_if = "Option::is_none")]
    dir: Option<&'a str>,
}

/// What [`write_viewer`] wrote, for the build's report.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ViewerSummary {
    pub nodes: usize,
    pub edges: usize,
    pub chapters: usize,
}

/// Writes `<output_dir>/graph.html` for `nodes` and their `graph` view.
pub fn write_viewer(
    nodes: &[NormalizedNode],
    graph: &GraphView,
    output_dir: &Path,
    generated_at: DateTime<Utc>,
) -> Result<ViewerSummary> {
    let (chapters, chapter_of) = chapters(nodes);
    let concepts: HashMap<&str, &NormalizedNode> = nodes.iter().map(|n| (n.id(), n)).collect();
    let view_nodes: Vec<Node> = graph
        .nodes
        .iter()
        .map(|v| match concepts.get(v.id.as_str()) {
            Some(node) => Node {
                id: &v.id,
                title: node.title(),
                type_: &v.type_,
                chapter: chapter_of.get(v.id.as_str()).copied(),
                role: v.processing_role.as_deref(),
                file: Some(node.source_file()),
                dir: Some(node.bundle_dir()),
            },
            None => Node {
                id: &v.id,
                title: v.title.as_deref().unwrap_or(&v.id),
                type_: &v.type_,
                chapter: None,
                role: None,
                file: v.source_file.as_deref(),
                dir: None,
            },
        })
        .collect();
    let title = nodes
        .iter()
        .filter(|n| matches!(n, NormalizedNode::Map(_)))
        .map(|n| n.title())
        .collect::<Vec<_>>()
        .join(", ");
    let payload = Payload {
        title: if title.is_empty() {
            "DITA2Graph".to_string()
        } else {
            title
        },
        generated: format!(
            "{} at {}",
            crate::okf::producer(),
            generated_at.format("%Y-%m-%d %H:%M UTC")
        ),
        chapters,
        nodes: view_nodes,
        edges: &graph.edges,
    };
    let summary = ViewerSummary {
        nodes: payload.nodes.len(),
        edges: payload.edges.len(),
        chapters: payload.chapters.len(),
    };
    let json = serde_json::to_string(&payload).context("serializing the viewer's data")?;
    let html = TEMPLATE.replacen(DATA_PLACEHOLDER, &script_safe(&json), 1);
    fs::write(output_dir.join(VIEWER_FILE), html)
        .with_context(|| format!("writing {}", output_dir.join(VIEWER_FILE).display()))?;
    Ok(summary)
}

/// Removes a `graph.html` an earlier `--viewer true` build wrote, so the
/// page never shows an older graph than `okf/`; one without the marker is
/// not this tool's and stays. Returns whether a page was removed.
pub fn remove_stale_viewer(output_dir: &Path) -> Result<bool> {
    let path = output_dir.join(VIEWER_FILE);
    match fs::read_to_string(&path) {
        Ok(content) if content.contains(VIEWER_MARKER) => {
            fs::remove_file(&path).with_context(|| format!("removing stale {}", path.display()))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// JSON inside `<script type="application/json">`: a `<` only occurs in
/// its strings, where `<` reads back the same, and a title quoting
/// `</script>` can then not close the element.
fn script_safe(json: &str) -> String {
    json.replace('<', "\\u003c")
}

/// The chapters, the top-level `contains` entries of the maps in map
/// order, and the chapter of every topic under one: the nearest chapter
/// down the `contains` edges (breadth first, the earlier chapter on a
/// tie), so a topic the map places twice goes where it sits highest.
fn chapters(nodes: &[NormalizedNode]) -> (Vec<Chapter>, HashMap<&str, usize>) {
    let by_id: HashMap<&str, &NormalizedNode> = nodes.iter().map(|n| (n.id(), n)).collect();
    let children = |id: &str| -> Vec<&str> {
        by_id
            .get(id)
            .map(|n| {
                n.links()
                    .iter()
                    .filter(|l| l.relation == Relation::Contains)
                    .map(|l| l.target.as_str())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut chapters = Vec::new();
    let mut chapter_of: HashMap<&str, usize> = HashMap::new();
    let mut queue = VecDeque::new();
    for map in nodes.iter().filter(|n| matches!(n, NormalizedNode::Map(_))) {
        for id in children(map.id()) {
            let Some(node) = by_id.get(id) else { continue };
            if chapter_of.contains_key(id) || matches!(node, NormalizedNode::Map(_)) {
                continue;
            }
            chapter_of.insert(id, chapters.len());
            queue.push_back(id);
            chapters.push(Chapter {
                id: id.to_string(),
                title: node.title().to_string(),
            });
        }
    }
    while let Some(id) = queue.pop_front() {
        let chapter = chapter_of[id];
        for child in children(id) {
            if by_id.contains_key(child) && !chapter_of.contains_key(child) {
                chapter_of.insert(child, chapter);
                queue.push_back(child);
            }
        }
    }
    (chapters, chapter_of)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_view::graph_view;
    use serde_json::{Value, json};

    fn nodes() -> Vec<NormalizedNode> {
        serde_json::from_value(json!([
            {"type": "map", "id": "guide", "title": "User Guide", "sourceFile": "guide.ditamap",
             "links": [{"relation": "contains", "target": "install"},
                       {"relation": "contains", "target": "configure"}],
             "keyDefs": [{"name": "site", "url": "https://example.org/", "sourceFile": "guide.ditamap"}]},
            {"type": "topic", "id": "install", "topicType": "task", "title": "Installing",
             "sourceFile": "topics/install.dita",
             "links": [{"relation": "contains", "target": "prereqs"},
                       {"relation": "references", "target": "settings"}]},
            {"type": "topic", "id": "configure", "topicType": "concept", "title": "Configuring",
             "sourceFile": "topics/configure.dita",
             "links": [{"relation": "contains", "target": "settings"},
                       {"relation": "contains", "target": "prereqs"}]},
            {"type": "topic", "id": "prereqs", "topicType": "topic", "title": "Prerequisites",
             "sourceFile": "topics/prereqs.dita", "links": []},
            {"type": "topic", "id": "settings", "topicType": "reference", "title": "Settings </script>",
             "sourceFile": "topics/settings.dita", "links": []},
            {"type": "topic", "id": "shared", "topicType": "topic", "title": "Shared text",
             "sourceFile": "topics/shared.dita", "processingRole": "resource-only", "links": [],
             "keyrefs": ["site"]}
        ]))
        .unwrap()
    }

    fn data(html: &str) -> Value {
        let start = html
            .find("<script id=\"graph-data\" type=\"application/json\">")
            .unwrap();
        let json = &html[start..];
        let json = &json[json.find('>').unwrap() + 1..json.find("</script>").unwrap()];
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_topic_takes_the_nearest_chapter_above_it() {
        let nodes = nodes();
        let (chapters, chapter_of) = chapters(&nodes);
        let titles: Vec<_> = chapters.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Installing", "Configuring"]);
        assert_eq!(chapter_of["install"], 0);
        assert_eq!(
            chapter_of["prereqs"], 0,
            "under both, first chapter on a tie"
        );
        assert_eq!(chapter_of["settings"], 1);
        assert!(!chapter_of.contains_key("shared"));
        assert!(!chapter_of.contains_key("guide"));
    }

    #[test]
    fn the_page_holds_every_node_and_edge_with_titles_and_chapters() {
        let nodes = nodes();
        let graph = graph_view(&nodes);
        let dir = tempfile::tempdir().unwrap();
        let at = "2026-11-19T10:00:00Z".parse().unwrap();
        let summary = write_viewer(&nodes, &graph, dir.path(), at).unwrap();
        assert_eq!(
            summary,
            ViewerSummary {
                nodes: graph.nodes.len(),
                edges: graph.edges.len(),
                chapters: 2
            }
        );

        let html = fs::read_to_string(dir.path().join(VIEWER_FILE)).unwrap();
        assert!(html.contains(VIEWER_MARKER));
        assert!(!html.contains(DATA_PLACEHOLDER));
        // A title quoting </script> does not end the data element.
        assert_eq!(
            html.matches("</script>").count(),
            2,
            "the data and the script"
        );
        let data = data(&html);
        assert_eq!(data["title"], "User Guide");
        assert!(
            data["generated"]
                .as_str()
                .unwrap()
                .ends_with("at 2026-11-19 10:00 UTC")
        );
        let node = |id: &str| {
            data["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(node("settings")["title"], "Settings </script>");
        assert_eq!(node("settings")["chapter"], 1);
        assert_eq!(node("settings")["dir"], "topics");
        assert_eq!(node("guide")["dir"], "maps");
        assert_eq!(node("shared")["role"], "resource-only");
        assert!(node("shared").get("chapter").is_none());
        assert_eq!(node("key:site")["type"], "Key");
        assert!(node("key:site").get("dir").is_none());
        assert_eq!(data["edges"].as_array().unwrap().len(), graph.edges.len());
    }

    #[test]
    fn only_a_page_this_tool_wrote_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!remove_stale_viewer(dir.path()).unwrap());
        fs::write(dir.path().join(VIEWER_FILE), "<html>someone else's</html>").unwrap();
        assert!(!remove_stale_viewer(dir.path()).unwrap());
        assert!(dir.path().join(VIEWER_FILE).exists());

        let nodes = nodes();
        let at = "2026-11-19T10:00:00Z".parse().unwrap();
        write_viewer(&nodes, &graph_view(&nodes), dir.path(), at).unwrap();
        assert!(remove_stale_viewer(dir.path()).unwrap());
        assert!(!dir.path().join(VIEWER_FILE).exists());
    }
}
