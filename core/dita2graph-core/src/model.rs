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
    #[serde(default)]
    pub audience: Vec<String>,
    #[serde(default)]
    pub product: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(rename = "sourceFile")]
    pub source_file: String,
    #[serde(default)]
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedMap {
    pub id: String,
    pub title: String,
    #[serde(rename = "sourceFile")]
    pub source_file: String,
    #[serde(default)]
    pub links: Vec<Link>,
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
    Topic,
}

impl TopicType {
    /// The OKF frontmatter `type` value, per the §4.1 mapping table.
    pub fn okf_type(&self) -> &'static str {
        match self {
            TopicType::Concept => "Concept",
            TopicType::Task => "Task",
            TopicType::Reference => "Reference",
            TopicType::Glossentry => "Glossary Entry",
            TopicType::Topic => "Topic",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Link {
    pub relation: Relation,
    pub target: String,
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
