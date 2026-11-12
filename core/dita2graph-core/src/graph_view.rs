//! The flattened node/edge view `graph.json` and `graph.db` share (§4.4).
//!
//! Besides the maps and topics (each with its OKF concept file) and their
//! links, the view holds what they declare, as nodes of their own so the
//! MCP server's traversals reach them: the map's keys (`key:<name>`,
//! `scope.name` for a scoped key), the URLs topics link to (the URL itself)
//! and the sample files they include (`file:<path>`). These nodes have no
//! concept file; they carry their title and source file instead.

use crate::model::NormalizedNode;
use serde::Serialize;
use std::collections::HashMap;

/// Node type of a key.
pub const KEY_TYPE: &str = "Key";
/// Node type of a URL outside the corpus.
pub const EXTERNAL_TYPE: &str = "External Resource";
/// Node type of a sample file a topic includes, or a local file a key names.
pub const FILE_TYPE: &str = "File";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewNode {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    /// Only on nodes without a concept file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Only on nodes without a concept file: the file that defines a key,
    /// or the file itself.
    #[serde(rename = "sourceFile", skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// `resource-only` on a topic the map uses only as a resource (a reuse
    /// warehouse, a key target): not a page of the publication.
    #[serde(rename = "processingRole", skip_serializing_if = "Option::is_none")]
    pub processing_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<String>,
}

/// The nodes and edges `graph.json` and `graph.db` hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GraphView {
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<ViewEdge>,
}

/// Whether a node of this type is one the graph declares without a
/// concept file: a key, an external URL, a file.
pub fn is_declared_type(type_: &str) -> bool {
    matches!(type_, KEY_TYPE | EXTERNAL_TYPE | FILE_TYPE)
}

/// The graph id of a key, as a topic's `@keyref` names it.
pub fn key_id(qualified_name: &str) -> String {
    format!("key:{qualified_name}")
}

/// The graph id of a file of the corpus that is not a topic.
pub fn file_id(path: &str) -> String {
    format!("file:{path}")
}

/// Every map and topic, then the keys, URLs and files they declare, each
/// node once in first-seen order. Two maps or topics sharing an id are
/// one node, the last one -- as `okf/` keeps the last concept file
/// written. The maps' key definitions come before the keys topics use,
/// whatever the order of `nodes`, so a key keeps its title and defining
/// file even when a topic using it is listed before the map.
pub fn graph_view(nodes: &[NormalizedNode]) -> GraphView {
    let mut view = View::default();
    for node in nodes {
        let role = match node {
            NormalizedNode::Topic(topic) => topic.processing_role.clone(),
            NormalizedNode::Map(_) => None,
        };
        view.concept(node.id(), node.okf_type(), role);
    }
    for node in nodes {
        if let NormalizedNode::Map(map) = node {
            for key in &map.key_defs {
                let name = key.qualified_name();
                let title = match &key.text {
                    Some(text) => format!("{name} = \"{text}\""),
                    None => name.clone(),
                };
                let id = key_id(&name);
                view.node(&id, KEY_TYPE, Some(title), Some(key.source_file.clone()));
                view.edge(&map.id, &id, "defines-key");
                let target = if let Some(target) = &key.target {
                    Some(target.clone())
                } else if let Some(url) = &key.url {
                    view.node(url, EXTERNAL_TYPE, Some(url.clone()), None);
                    Some(url.clone())
                } else if let Some(path) = &key.file {
                    let file = file_id(path);
                    view.node(&file, FILE_TYPE, Some(path.clone()), Some(path.clone()));
                    Some(file)
                } else {
                    None
                };
                if let Some(target) = target {
                    view.edge(&id, &target, "key-target");
                }
            }
        }
    }
    for node in nodes {
        for link in node.links() {
            view.edges.push(ViewEdge {
                from: node.id().to_string(),
                to: link.target.clone(),
                relation: link.relation.as_str().to_string(),
                fragments: link.fragments.clone(),
            });
        }
        match node {
            NormalizedNode::Map(_) => {}
            NormalizedNode::Topic(topic) => {
                for name in &topic.keyrefs {
                    let id = key_id(name);
                    // A key no map defines: still a node, titled by its name.
                    view.node(&id, KEY_TYPE, Some(name.clone()), None);
                    view.edge(&topic.id, &id, "uses-key");
                }
                for link in &topic.external_links {
                    view.node(&link.url, EXTERNAL_TYPE, Some(link.url.clone()), None);
                    view.edge(&topic.id, &link.url, "links-to");
                }
                for path in &topic.code_refs {
                    let file = file_id(path);
                    view.node(&file, FILE_TYPE, Some(path.clone()), Some(path.clone()));
                    view.edge(&topic.id, &file, "includes-code");
                }
            }
        }
    }
    GraphView {
        nodes: view.nodes,
        edges: view.edges,
    }
}

#[derive(Default)]
struct View {
    nodes: Vec<ViewNode>,
    index: HashMap<String, usize>,
    edges: Vec<ViewEdge>,
}

impl View {
    /// A map or topic: a later one with the same id replaces it in place.
    fn concept(&mut self, id: &str, type_: &str, processing_role: Option<String>) {
        let node = ViewNode {
            id: id.to_string(),
            type_: type_.to_string(),
            title: None,
            source_file: None,
            processing_role,
        };
        match self.index.get(id) {
            Some(&i) => self.nodes[i] = node,
            None => {
                self.index.insert(id.to_string(), self.nodes.len());
                self.nodes.push(node);
            }
        }
    }

    /// A key, URL or file: the first mention makes the node; a later one
    /// only fills in a title or source file still missing.
    fn node(&mut self, id: &str, type_: &str, title: Option<String>, source_file: Option<String>) {
        match self.index.get(id) {
            Some(&i) => {
                let node = &mut self.nodes[i];
                if node.source_file.is_none() {
                    node.source_file = source_file;
                }
                if node.title.is_none() {
                    node.title = title;
                }
            }
            None => {
                self.index.insert(id.to_string(), self.nodes.len());
                self.nodes.push(ViewNode {
                    id: id.to_string(),
                    type_: type_.to_string(),
                    title,
                    source_file,
                    processing_role: None,
                });
            }
        }
    }

    fn edge(&mut self, from: &str, to: &str, relation: &str) {
        self.edges.push(ViewEdge {
            from: from.to_string(),
            to: to.to_string(),
            relation: relation.to_string(),
            fragments: Vec::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ExternalLink, KeyDef, NormalizedMap, NormalizedTopic, TopicType};

    fn nodes() -> Vec<NormalizedNode> {
        let key = |name: &str| KeyDef {
            name: name.into(),
            scope: None,
            target: None,
            url: None,
            file: None,
            text: None,
            source_file: "keys.ditamap".into(),
        };
        vec![
            NormalizedNode::Map(NormalizedMap {
                id: "guide".into(),
                title: "Guide".into(),
                metadata: Default::default(),
                source_file: "guide.ditamap".into(),
                key_defs: vec![
                    KeyDef {
                        target: Some("install".into()),
                        ..key("install")
                    },
                    KeyDef {
                        url: Some("https://brew.sh".into()),
                        ..key("brew")
                    },
                    KeyDef {
                        text: Some("Acme".into()),
                        scope: Some("legacy".into()),
                        ..key("product")
                    },
                ],
                links: vec![],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "install".into(),
                topic_type: TopicType::Task,
                title: "Install".into(),
                shortdesc: None,
                body: None,
                resolved_body: None,
                processing_role: Some("resource-only".into()),
                dita_class: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                keyrefs: vec!["brew".into(), "undefined".into()],
                external_links: vec![ExternalLink {
                    url: "https://brew.sh".into(),
                    key: Some("brew".into()),
                }],
                code_refs: vec!["samples/build.xml".into()],
                authored_file: None,
                metadata: Default::default(),
                source_file: "topics/install.dita".into(),
                links: vec![],
            }),
        ]
    }

    #[test]
    fn two_nodes_sharing_an_id_are_the_last_one_as_in_okf() {
        let mut nodes = nodes();
        let NormalizedNode::Topic(mut copy) = nodes[1].clone() else {
            panic!("a topic")
        };
        copy.topic_type = TopicType::Concept;
        copy.processing_role = None;
        nodes.push(NormalizedNode::Topic(copy));
        let view = graph_view(&nodes);
        let install: Vec<&ViewNode> = view.nodes.iter().filter(|n| n.id == "install").collect();
        assert_eq!(install.len(), 1);
        assert_eq!(install[0].type_, "Concept", "the later node wins");
        assert_eq!(install[0].processing_role, None);
        assert_eq!(view.nodes[1].id, "install", "in the first one's place");
    }

    #[test]
    fn keys_urls_and_files_become_nodes_once_with_their_edges() {
        let GraphView { nodes, edges } = graph_view(&nodes());
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "guide",
                "install",
                "key:install",
                "key:brew",
                "https://brew.sh",
                "key:legacy.product",
                "key:undefined",
                "file:samples/build.xml"
            ]
        );
        // The same view whatever the order of the model: the map's
        // definition still titles the key a topic listed first uses.
        let mut reversed = super::tests::nodes();
        reversed.reverse();
        let brew_reversed = graph_view(&reversed)
            .nodes
            .into_iter()
            .find(|n| n.id == "key:brew")
            .unwrap();
        assert_eq!(brew_reversed.source_file.as_deref(), Some("keys.ditamap"));
        let brew = &nodes[3];
        assert_eq!(brew.type_, KEY_TYPE);
        assert_eq!(brew.source_file.as_deref(), Some("keys.ditamap"));
        assert_eq!(nodes[5].title.as_deref(), Some("legacy.product = \"Acme\""));
        // Maps and topics keep their concept file: no title of their own.
        assert_eq!(nodes[1].title, None);
        // A resource-only topic says so; the other nodes carry no role.
        assert_eq!(nodes[1].processing_role.as_deref(), Some("resource-only"));
        assert!(
            nodes
                .iter()
                .filter(|n| n.id != "install")
                .all(|n| n.processing_role.is_none())
        );

        let edges: Vec<String> = edges
            .iter()
            .map(|e| format!("{} -{}-> {}", e.from, e.relation, e.to))
            .collect();
        assert_eq!(
            edges,
            [
                "guide -defines-key-> key:install",
                "key:install -key-target-> install",
                "guide -defines-key-> key:brew",
                "key:brew -key-target-> https://brew.sh",
                "guide -defines-key-> key:legacy.product",
                "install -uses-key-> key:brew",
                "install -uses-key-> key:undefined",
                "install -links-to-> https://brew.sh",
                "install -includes-code-> file:samples/build.xml",
            ]
        );
    }
}
