//! The normalized DITA model: the JSON contract between the DITA-OT Java
//! plugin (§2 of `docs/plugin-specification.md`) and this Rust core
//! engine (§3.2). This is the *only* interface the two languages share —
//! everything downstream of `NormalizedNode` is pure Rust.

use serde::{Deserialize, Serialize};

/// One node of the resolved DITA model: either a topic or a ditamap.
/// Internally tagged on `type` so the wire format matches
/// `docs/plugin-specification.md` §3.2 exactly (`{"type": "topic", ...}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
// Nearly every node is a topic (a corpus has a handful of maps), so
// boxing topics to shrink the map variant would cost one allocation per
// node to save memory on almost none.
#[allow(clippy::large_enum_variant)]
pub enum NormalizedNode {
    Topic(NormalizedTopic),
    Map(NormalizedMap),
}

impl NormalizedNode {
    pub fn id(&self) -> &str {
        match self {
            NormalizedNode::Topic(t) => &t.id,
            NormalizedNode::Map(m) => &m.id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            NormalizedNode::Topic(t) => &t.title,
            NormalizedNode::Map(m) => &m.title,
        }
    }

    pub fn links(&self) -> &[Link] {
        match self {
            NormalizedNode::Topic(t) => &t.links,
            NormalizedNode::Map(m) => &m.links,
        }
    }

    pub fn source_file(&self) -> &str {
        match self {
            NormalizedNode::Topic(t) => &t.source_file,
            NormalizedNode::Map(m) => &m.source_file,
        }
    }

    /// The OKF frontmatter `type` value for this node (§4.1's DITA→OKF
    /// mapping table).
    pub fn okf_type(&self) -> &'static str {
        match self {
            NormalizedNode::Topic(t) => t.topic_type.okf_type(),
            NormalizedNode::Map(_) => "DITA Map",
        }
    }

    /// The bundle subdirectory this node's concept file belongs in
    /// (`okf/topics/` or `okf/maps/`, §2.4).
    pub fn bundle_dir(&self) -> &'static str {
        match self {
            NormalizedNode::Topic(_) => "topics",
            NormalizedNode::Map(_) => "maps",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedTopic {
    pub id: String,
    #[serde(rename = "topicType")]
    pub topic_type: TopicType,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortdesc: Option<String>,
    /// Whitespace-normalized body text (`conbody`/`taskbody`/`refbody`/
    /// `glossdef`/generic `body`), markup stripped -- the "cleaned text"
    /// input for both the OKF bundle's body content and the RAG index
    /// (§4.4, §13.1). Distinct from `shortdesc`, a separate sibling
    /// element in DITA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// `body` as a reader of the published topic sees it: every fragment
    /// pulled in by `conref`/`conkeyref` kept in place, between
    /// `[reused from <file>#<id>]` and `[end reused]` markers. Only set
    /// when the topic reuses something. Never rendered into the OKF
    /// concept (whose `# Content` stays deduplicated) -- it feeds the RAG
    /// chunk's `resolvedText`, which `read_topic` returns in full.
    #[serde(
        default,
        rename = "resolvedBody",
        skip_serializing_if = "Option::is_none"
    )]
    pub resolved_body: Option<String>,
    /// `resource-only` when the map uses the topic only as a resource
    /// (a reuse warehouse, a key target) and never publishes it in the
    /// table of contents; `None` for a normal topic.
    #[serde(
        default,
        rename = "processingRole",
        skip_serializing_if = "Option::is_none"
    )]
    pub processing_role: Option<String>,
    /// The root element's DITA `@class` (`- topic/topic concept/concept `
    /// trimmed), which names specializations the topic type folds away.
    #[serde(default, rename = "ditaClass", skip_serializing_if = "Option::is_none")]
    pub dita_class: Option<String>,
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub product: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    /// Distinct `<uicontrol>` text found anywhere in this topic's body
    /// (§3.3's `applies-to` inference target side, `relations.rs`) --
    /// what a reference topic "defines".
    #[serde(default)]
    pub uicontrols: Vec<String>,
    /// Distinct `<uicontrol>` text found specifically inside `<cmd>`
    /// elements (§3.3's `applies-to` inference source side) -- what a
    /// task step "invokes".
    #[serde(default, rename = "cmdUicontrols")]
    pub cmd_uicontrols: Vec<String>,
    /// Distinct names of the keys the topic uses: `@keyref` on any
    /// element, and `@conkeyref` when the topic really reuses the key's
    /// target. A name may be scope-qualified (`scope.key`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyrefs: Vec<String>,
    /// The `http(s)` URLs the topic links to, each once.
    #[serde(
        default,
        rename = "externalLinks",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub external_links: Vec<ExternalLink>,
    /// Paths of the sample files the topic includes with `<coderef>`,
    /// as DITA-OT's job lists them (relative to the source root).
    #[serde(default, rename = "codeRefs", skip_serializing_if = "Vec::is_empty")]
    pub code_refs: Vec<String>,
    #[serde(rename = "sourceFile")]
    pub source_file: String,
    /// The file the author edits, when DITA-OT wrote the topic under
    /// another name (a `copy-to` target, a branch-filtering copy):
    /// `source_file` is then the copy's own path, which stays the
    /// concept's identity. Written to the OKF concept as `sources`.
    #[serde(
        default,
        rename = "authoredFile",
        skip_serializing_if = "Option::is_none"
    )]
    pub authored_file: Option<String>,
    #[serde(default)]
    pub links: Vec<Link>,
}

/// A link from a topic to a URL outside the corpus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalLink {
    pub url: String,
    /// The key the link went through, when it used one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedMap {
    pub id: String,
    pub title: String,
    #[serde(rename = "sourceFile")]
    pub source_file: String,
    /// The effective key definitions of the map and its submaps, in map
    /// order (the first definition of a key in its scope wins).
    #[serde(default, rename = "keyDefs", skip_serializing_if = "Vec::is_empty")]
    pub key_defs: Vec<KeyDef>,
    #[serde(default)]
    pub links: Vec<Link>,
}

/// One key definition. At most one of `target`, `url` and `file` is set;
/// a key with none of them is a text key (a variable) whose value is
/// `text`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyDef {
    pub name: String,
    /// Dotted key scope; `None` for the root scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Id of the topic the key names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// URL of the external resource the key names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Path of the local non-topic file the key names (an image, a PDF).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The key's text (keyword, link text or navtitle) when it names no topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The map file that declares the key: the submap, when it is one.
    #[serde(rename = "sourceFile")]
    pub source_file: String,
}

impl KeyDef {
    /// The name a topic's `@keyref` uses for this key: `scope.name`, or
    /// the bare name in the root scope.
    pub fn qualified_name(&self) -> String {
        match &self.scope {
            Some(scope) => format!("{scope}.{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// DITA topic type (§1.3 glossary). `Topic` is the fallback for a plain
/// `<topic>` or an unrecognized/custom type — emitted as a generic OKF
/// concept per the spec's graceful-degradation rule (§4.1), and flagged
/// via `DITA2GRAPH040W` (§2.5) by the caller when it's used.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TopicType {
    Concept,
    Task,
    Reference,
    Glossentry,
    Troubleshooting,
    Topic,
    /// A table-of-contents heading with no topic of its own (a
    /// `topichead`, a navtitle-only bookmap `chapter`): a node so the
    /// graph keeps the outline, and the container of its children.
    Topichead,
}

impl TopicType {
    /// The OKF frontmatter `type` value, per the §4.1 mapping table.
    pub fn okf_type(&self) -> &'static str {
        match self {
            TopicType::Concept => "Concept",
            TopicType::Task => "Task",
            TopicType::Reference => "Reference",
            TopicType::Glossentry => "Glossary Entry",
            TopicType::Troubleshooting => "Troubleshooting",
            TopicType::Topic => "Topic",
            TopicType::Topichead => "Topic Head",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Link {
    pub relation: Relation,
    pub target: String,
    /// On `generated-from` only: the ids of the reused elements in the
    /// target topic (`plug-in`, or `li@37` when the element has no id),
    /// as the Java extractor resolved them from DITA-OT's `xtrc` traces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<String>,
}

/// The DITA relation taxonomy (§4.3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Relation {
    Contains,
    References,
    RelatedTo,
    AppliesTo,
    Requires,
    GeneratedFrom,
    /// From a `copy-to` copy to the topic it copies.
    CopyOf,
}

impl Relation {
    /// The `relations` frontmatter key / `graph.json` edge label (§4.1,
    /// §4.4) — matches the wire representation exactly since both use
    /// kebab-case.
    pub fn as_str(&self) -> &'static str {
        match self {
            Relation::Contains => "contains",
            Relation::References => "references",
            Relation::RelatedTo => "related-to",
            Relation::AppliesTo => "applies-to",
            Relation::Requires => "requires",
            Relation::GeneratedFrom => "generated-from",
            Relation::CopyOf => "copy-of",
        }
    }

    /// The markdown body section heading used when rendering this
    /// relation's targets as links (§4.4).
    pub fn section_heading(&self) -> &'static str {
        match self {
            Relation::Contains => "Contains",
            Relation::References => "References",
            Relation::RelatedTo => "Related",
            Relation::AppliesTo => "Applies To",
            Relation::Requires => "Requires",
            Relation::GeneratedFrom => "Generated From",
            Relation::CopyOf => "Copy Of",
        }
    }

    /// Whether this relation is additionally captured in the `relations`
    /// frontmatter extension (§4.1: "the typed, directional relations
    /// DITA is stricter about"), on top of the body markdown link every
    /// relation gets. `references`/`related-to`/`generated-from` are
    /// left as plain links only, since OKF already defines those
    /// natively and a frontmatter mirror would be redundant.
    pub fn needs_frontmatter_extension(&self) -> bool {
        matches!(
            self,
            Relation::Contains | Relation::AppliesTo | Relation::Requires
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_node_matches_spec_3_2_wire_format() {
        let json = r#"{
            "type": "topic",
            "id": "installing-product",
            "topicType": "task",
            "title": "Installing Product",
            "shortdesc": "Steps to install the product in a production environment.",
            "body": "Download the installer package for your platform. Run the installer.",
            "audience": ["admin"],
            "product": ["enterprise"],
            "keys": ["install-task"],
            "sourceFile": "topics/installing-product.dita",
            "links": [
                { "relation": "requires", "target": "configuration" },
                { "relation": "contains", "target": "installing-product-prereqs" }
            ]
        }"#;
        let node: NormalizedNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.id(), "installing-product");
        assert_eq!(node.okf_type(), "Task");
        assert_eq!(node.links().len(), 2);
        assert_eq!(node.links()[0].relation.as_str(), "requires");
        match &node {
            NormalizedNode::Topic(t) => assert_eq!(
                t.body.as_deref(),
                Some("Download the installer package for your platform. Run the installer.")
            ),
            NormalizedNode::Map(_) => panic!("expected a topic"),
        }
    }

    #[test]
    fn topichead_heading_node_reads_as_a_topic_head() {
        let json = r#"{
            "type": "topic",
            "id": "topichead--glossary",
            "topicType": "topichead",
            "title": "Glossary",
            "sourceFile": "UserManual.ditamap#topichead--glossary",
            "links": [{ "relation": "contains", "target": "anchor" }]
        }"#;
        let node: NormalizedNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.okf_type(), "Topic Head");
        assert_eq!(node.bundle_dir(), "topics");
    }

    #[test]
    fn generated_from_fragments_and_resolved_body_match_the_java_wire_format() {
        let json = r#"{
            "type": "topic",
            "id": "plugins-installing",
            "topicType": "task",
            "title": "Installing plug-ins",
            "body": "At the command-line prompt, enter the following command:",
            "resolvedBody": "At the prompt: [reused from resources/conref-task.dita#plug-in] the plug-in [end reused]",
            "sourceFile": "topics/plugins-installing.dita",
            "links": [
                { "relation": "generated-from", "target": "ID", "fragments": ["plug-in", "plugin-integrate-all"] },
                { "relation": "requires", "target": "configuration" }
            ]
        }"#;
        let node: NormalizedNode = serde_json::from_str(json).unwrap();
        let NormalizedNode::Topic(t) = &node else {
            panic!("expected a topic")
        };
        assert!(t.resolved_body.as_deref().unwrap().contains("#plug-in]"));
        assert_eq!(
            t.links[0].fragments,
            vec!["plug-in", "plugin-integrate-all"]
        );
        assert!(
            t.links[1].fragments.is_empty(),
            "fragments default to empty"
        );
        // And back out: no empty "fragments" key on ordinary links.
        let out = serde_json::to_string(&node).unwrap();
        assert_eq!(out.matches("\"fragments\"").count(), 1, "{out}");
    }

    #[test]
    fn body_is_optional_for_backward_compatibility_with_older_normalized_models() {
        let json = r#"{
            "type": "topic",
            "id": "configuration",
            "topicType": "concept",
            "title": "Configuration Overview",
            "sourceFile": "topics/configuration.dita",
            "links": []
        }"#;
        let node: NormalizedNode = serde_json::from_str(json).unwrap();
        match &node {
            NormalizedNode::Topic(t) => assert_eq!(t.body, None),
            NormalizedNode::Map(_) => panic!("expected a topic"),
        }
    }

    #[test]
    fn map_node_has_no_topic_type() {
        let json = r#"{
            "type": "map",
            "id": "user-guide",
            "title": "User Guide",
            "sourceFile": "user-guide.ditamap",
            "links": [
                { "relation": "contains", "target": "installing-product" },
                { "relation": "contains", "target": "configuration" }
            ]
        }"#;
        let node: NormalizedNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.okf_type(), "DITA Map");
        assert_eq!(node.bundle_dir(), "maps");
    }
}
