//! Writes a conformant OKF v0.2 bundle from normalized DITA nodes.
//!
//! Deliberately **not** built on `okf-generator`/`okf_parser::Concept`:
//! Phase 0 (see `docs/dev/phase-0-findings.md`) found that crate's data
//! model is hardcoded to a source-code vocabulary (`ConceptKind::{Package,
//! Module, Class, Function, ...}`, `RelationKind::{Calls, Imports, ...}`)
//! that cannot represent DITA topic types (`Task`/`Reference`/`Concept`/
//! `Glossary Entry`) or the DITA relation taxonomy (§4.3) without upstream
//! changes. The OKF v0.2 *format* itself is just markdown + YAML
//! frontmatter with one required key (`type`), so writing it directly
//! here is fully conformant — confirmed by round-tripping through
//! `okf_validator::validate_bundle`, which *is* reused as-is (§3, §6.4)
//! since it operates on raw parsed frontmatter, not the typed model.

use crate::model::{NormalizedNode, Relation};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// `generated.by` actor identity (OKF spec §7's `<producer>/<version>`
/// convention), matching the `dita2graph-core/0.1.0` shown in
/// `docs/plugin-specification.md` §4.4.
pub fn producer() -> String {
    format!("dita2graph-core/{}", env!("CARGO_PKG_VERSION"))
}

#[derive(Serialize)]
struct Frontmatter {
    #[serde(rename = "type")]
    type_: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    resource: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    generated: Generated,
    #[serde(skip_serializing_if = "Option::is_none")]
    relations: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Serialize)]
struct Generated {
    by: String,
    at: DateTime<Utc>,
}

/// A written bundle's summary, for CLI reporting (`dita2graph-core
/// build`, §3.4).
#[derive(Debug, Default)]
pub struct BundleSummary {
    pub topics_written: usize,
    pub maps_written: usize,
    pub edges_written: usize,
}

/// Writes `nodes` to `<output_dir>/okf/` as an OKF v0.2 bundle, plus the
/// derived `<output_dir>/graph.json` flattened view (§2.4, §4.4).
pub fn write_bundle(
    nodes: &[NormalizedNode],
    output_dir: &Path,
    generated_at: DateTime<Utc>,
) -> Result<BundleSummary> {
    let bundle_dir = output_dir.join("okf");
    fs::create_dir_all(bundle_dir.join("topics")).context("creating okf/topics")?;
    fs::create_dir_all(bundle_dir.join("maps")).context("creating okf/maps")?;

    // id -> (bundle_dir subdir, title), for cross-linking and section
    // rendering.
    let index: BTreeMap<&str, (&str, &str)> = nodes
        .iter()
        .map(|n| (n.id(), (n.bundle_dir(), n.title())))
        .collect();

    let mut summary = BundleSummary::default();
    for node in nodes {
        let subdir = node.bundle_dir();
        let path = bundle_dir.join(subdir).join(format!("{}.md", node.id()));
        let content = render_concept(node, &index, generated_at)?;
        fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        match node {
            NormalizedNode::Topic(_) => summary.topics_written += 1,
            NormalizedNode::Map(_) => summary.maps_written += 1,
        }
        summary.edges_written += node.links().len();
    }

    write_okf_toml(&bundle_dir)?;
    write_index(&bundle_dir, nodes, generated_at)?;
    write_graph_json(output_dir, nodes)?;

    Ok(summary)
}

fn render_concept(
    node: &NormalizedNode,
    index: &BTreeMap<&str, (&str, &str)>,
    generated_at: DateTime<Utc>,
) -> Result<String> {
    let description = match node {
        NormalizedNode::Topic(t) => t.shortdesc.clone(),
        NormalizedNode::Map(_) => None,
    };
    let tags = match node {
        NormalizedNode::Topic(t) => {
            let mut tags: Vec<String> =
                t.audience.iter().chain(t.product.iter()).cloned().collect();
            tags.extend(t.keys.iter().cloned());
            tags
        }
        NormalizedNode::Map(_) => Vec::new(),
    };

    let mut relations: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for link in node.links() {
        if link.relation.needs_frontmatter_extension() {
            relations
                .entry(link.relation.as_str().to_string())
                .or_default()
                .push(link.target.clone());
        }
    }

    let frontmatter = Frontmatter {
        type_: node.okf_type().to_string(),
        title: node.title().to_string(),
        description: description.clone(),
        resource: node.source_file().to_string(),
        tags,
        generated: Generated {
            by: producer(),
            at: generated_at,
        },
        relations: if relations.is_empty() {
            None
        } else {
            Some(relations)
        },
    };

    let yaml = serde_yaml::to_string(&frontmatter).context("serializing frontmatter")?;

    let mut body = String::new();
    if let Some(desc) = &description {
        body.push_str("# Summary\n\n");
        body.push_str(desc);
        body.push_str("\n\n");
    }

    // Group links by relation, preserving first-seen order within a
    // relation, so each relation the node actually has gets exactly one
    // section (§4.4's "# Requires" / "# Contains" pattern).
    let mut by_relation: Vec<(Relation, Vec<&str>)> = Vec::new();
    for link in node.links() {
        if let Some(entry) = by_relation.iter_mut().find(|(r, _)| *r == link.relation) {
            entry.1.push(&link.target);
        } else {
            by_relation.push((link.relation, vec![&link.target]));
        }
    }

    for (relation, targets) in by_relation {
        body.push_str(&format!("# {}\n\n", relation.section_heading()));
        for target in targets {
            let (target_dir, target_title) =
                index.get(target).copied().unwrap_or(("topics", target));
            let link = relative_link(node.bundle_dir(), target_dir, target);
            body.push_str(&format!("- [{target_title}]({link})\n"));
        }
        body.push('\n');
    }

    Ok(format!("---\n{yaml}---\n\n{}", body.trim_end()))
}

/// A markdown link from a concept in `from_dir` (`"topics"`/`"maps"`) to
/// `target_id` in `to_dir`, relative to the linking file's own location.
fn relative_link(from_dir: &str, to_dir: &str, target_id: &str) -> String {
    if from_dir == to_dir {
        format!("{target_id}.md")
    } else {
        format!("../{to_dir}/{target_id}.md")
    }
}

fn write_okf_toml(bundle_dir: &Path) -> Result<()> {
    let content = format!(
        "okf_version = \"0.2\"\ngenerator = \"{}\"\noutput = \".\"\n",
        producer()
    );
    fs::write(bundle_dir.join("okf.toml"), content).context("writing okf.toml")
}

/// The bundle-root `index.md`. Per `okf_validator::check_required_index`
/// this is mandatory, and per `check_index_frontmatter` only the root
/// `index.md` may carry an `okf_version` declaration — every other
/// `index.md` in a bundle (this one has none) must have none at all.
fn write_index(
    bundle_dir: &Path,
    nodes: &[NormalizedNode],
    generated_at: DateTime<Utc>,
) -> Result<()> {
    let mut body = String::new();
    body.push_str("---\nokf_version: \"0.2\"\n---\n\n");
    body.push_str("# DITA2Graph knowledge bundle\n\n");
    body.push_str(&format!(
        "Generated {} by {}.\n\n",
        generated_at.to_rfc3339(),
        producer()
    ));

    body.push_str("## Maps\n\n");
    for node in nodes.iter().filter(|n| matches!(n, NormalizedNode::Map(_))) {
        body.push_str(&format!("- [{}](maps/{}.md)\n", node.title(), node.id()));
    }
    body.push('\n');

    body.push_str("## Topics\n\n");
    for node in nodes
        .iter()
        .filter(|n| matches!(n, NormalizedNode::Topic(_)))
    {
        body.push_str(&format!("- [{}](topics/{}.md)\n", node.title(), node.id()));
    }
    body.push('\n');

    fs::write(bundle_dir.join("index.md"), body).context("writing index.md")
}

// `log.md` (OKF spec §9, "chronological history of updates") is
// deliberately not written yet: `okf-validator` v0.3.0 doesn't implement
// the spec's reserved-filename exemption for it the way it does for
// `index.md` (see `is_index`), so a `log.md` without concept-shaped
// frontmatter fails validation as an orphaned, frontmatter-less concept.
// Tracked in docs/dev/phase-0-findings.md; re-add once upstream handles
// it, rather than shipping a bundle that fails our own validation gate.

#[derive(Serialize)]
struct GraphJson {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

#[derive(Serialize)]
struct GraphNode {
    id: String,
    #[serde(rename = "type")]
    type_: String,
}

#[derive(Serialize)]
struct GraphEdge {
    from: String,
    to: String,
    relation: String,
}

fn write_graph_json(output_dir: &Path, nodes: &[NormalizedNode]) -> Result<()> {
    let graph = GraphJson {
        nodes: nodes
            .iter()
            .map(|n| GraphNode {
                id: n.id().to_string(),
                type_: n.okf_type().to_string(),
            })
            .collect(),
        edges: nodes
            .iter()
            .flat_map(|n| {
                n.links().iter().map(move |l| GraphEdge {
                    from: n.id().to_string(),
                    to: l.target.clone(),
                    relation: l.relation.as_str().to_string(),
                })
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&graph).context("serializing graph.json")?;
    fs::write(output_dir.join("graph.json"), json).context("writing graph.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Link, NormalizedMap, NormalizedTopic, TopicType};
    use okf_validator::validate_bundle;

    fn sample_nodes() -> Vec<NormalizedNode> {
        vec![
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
                        target: "configuration".into(),
                    },
                ],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "installing-product".into(),
                topic_type: TopicType::Task,
                title: "Installing Product".into(),
                shortdesc: Some("Steps to install the product in a production environment.".into()),
                audience: vec!["admin".into()],
                product: vec!["enterprise".into()],
                keys: vec!["install-task".into()],
                source_file: "topics/installing-product.dita".into(),
                links: vec![
                    Link {
                        relation: Relation::Requires,
                        target: "configuration".into(),
                    },
                    Link {
                        relation: Relation::Contains,
                        target: "installing-product-prereqs".into(),
                    },
                ],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "installing-product-prereqs".into(),
                topic_type: TopicType::Topic,
                title: "Installing Product: Prerequisites".into(),
                shortdesc: None,
                audience: vec!["admin".into()],
                product: vec!["enterprise".into()],
                keys: vec![],
                source_file: "topics/installing-product-prereqs.dita".into(),
                links: vec![Link {
                    relation: Relation::References,
                    target: "configuration".into(),
                }],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "configuration".into(),
                topic_type: TopicType::Concept,
                title: "Configuration Overview".into(),
                shortdesc: None,
                audience: vec![],
                product: vec![],
                keys: vec!["config-concept".into()],
                source_file: "topics/configuration.dita".into(),
                links: vec![],
            }),
        ]
    }

    #[test]
    fn written_bundle_passes_okf_validator() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();

        let summary = write_bundle(&nodes, dir.path(), generated_at).unwrap();
        assert_eq!(summary.maps_written, 1);
        assert_eq!(summary.topics_written, 3);

        let report = validate_bundle(&dir.path().join("okf")).unwrap();
        assert!(
            !report.has_errors(),
            "expected a conformant bundle, got issues: {:#?}",
            report.issues
        );
    }

    #[test]
    fn task_frontmatter_carries_requires_and_contains_but_not_references() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at).unwrap();

        let task = fs::read_to_string(dir.path().join("okf/topics/installing-product.md")).unwrap();
        assert!(task.contains("type: Task"));
        assert!(task.contains("requires:\n  - configuration"));
        assert!(task.contains("- installing-product-prereqs"));
        assert!(task.contains("# Requires"));
        assert!(task.contains("[Configuration Overview](configuration.md)"));

        let prereqs =
            fs::read_to_string(dir.path().join("okf/topics/installing-product-prereqs.md"))
                .unwrap();
        // `references` is a plain body link, not a frontmatter `relations` entry.
        assert!(!prereqs.contains("relations:"));
        assert!(prereqs.contains("# References"));
    }

    #[test]
    fn map_links_to_topics_across_directories() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at).unwrap();

        let map = fs::read_to_string(dir.path().join("okf/maps/user-guide.md")).unwrap();
        assert!(map.contains("../topics/installing-product.md"));
        assert!(map.contains("../topics/configuration.md"));
    }

    #[test]
    fn graph_json_is_a_flattened_view() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at).unwrap();

        let graph: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("graph.json")).unwrap())
                .unwrap();
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 4);
        assert!(
            graph["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["from"] == "installing-product"
                    && e["to"] == "configuration"
                    && e["relation"] == "requires")
        );
    }
}
