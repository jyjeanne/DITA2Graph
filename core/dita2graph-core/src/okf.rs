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

use crate::model::{Link, NormalizedNode, Relation};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
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
    /// Concept files left untouched because the node (and every node it
    /// links to, title/subdirectory-wise) was unchanged since the last
    /// build (`crate::incremental`, Phase 6+'s "Incremental rebuild").
    /// Disjoint from `topics_written`/`maps_written` -- every node is
    /// counted in exactly one of the three.
    pub concepts_unchanged: usize,
}

/// Writes `nodes` to `<output_dir>/okf/` as an OKF v0.2 bundle, plus
/// (when `emit_graph_json` is true) the derived
/// `<output_dir>/graph.json` flattened view (§2.3's
/// `args.dita2graph.emit-graph-json`, default `true`; §2.4, §4.4).
///
/// Self-contained incremental rebuild: loads `<output_dir>/build-state.json`
/// (the previous build's fingerprints) at the start and writes a fresh
/// one at the end, entirely internally -- callers never see this and the
/// signature never changed, so every existing call site behaves exactly
/// as before (a first build in a fresh directory has nothing to skip
/// against anyway). A node whose fingerprint is unchanged and whose
/// concept file already exists keeps that file untouched rather than
/// re-rendering it with a new `generated.at` timestamp for no other
/// reason than "a build ran" -- see `crate::incremental`'s docs for why
/// this needs more than just the node's own fields.
pub fn write_bundle(
    nodes: &[NormalizedNode],
    output_dir: &Path,
    generated_at: DateTime<Utc>,
    emit_graph_json: bool,
) -> Result<BundleSummary> {
    let graph = emit_graph_json.then(|| crate::graph_view::graph_view(nodes));
    write_bundle_with(nodes, output_dir, generated_at, graph.as_ref())
}

/// [`write_bundle`] with the node/edge view already built, so a build
/// that also writes `graph.db` builds it once; `graph.json` is written
/// when `graph` is given.
pub fn write_bundle_with(
    nodes: &[NormalizedNode],
    output_dir: &Path,
    generated_at: DateTime<Utc>,
    graph: Option<&crate::graph_view::GraphView>,
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

    let previous_state = crate::incremental::load_state(output_dir);

    let mut summary = BundleSummary::default();
    for node in nodes {
        let subdir = node.bundle_dir();
        let path = bundle_dir.join(subdir).join(format!("{}.md", node.id()));
        if crate::incremental::can_skip(&previous_state, node, &index, &path)? {
            summary.concepts_unchanged += 1;
        } else {
            let content = render_concept(node, &index, generated_at)?;
            fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
            match node {
                NormalizedNode::Topic(_) => summary.topics_written += 1,
                NormalizedNode::Map(_) => summary.maps_written += 1,
            }
        }
        summary.edges_written += node.links().len();
    }

    crate::incremental::write_state(nodes, output_dir)?;

    write_okf_toml(&bundle_dir)?;
    write_index(&bundle_dir, nodes, generated_at)?;
    if let Some(graph) = graph {
        write_graph_json(output_dir, graph)?;
    }

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
        body.push_str(&escape_link_syntax(desc));
        body.push_str("\n\n");
    }
    if let NormalizedNode::Topic(t) = node
        && let Some(content) = &t.body
    {
        body.push_str("# Content\n\n");
        body.push_str(&escape_link_syntax(content));
        body.push_str("\n\n");
    }

    // Group links by relation, preserving first-seen order within a
    // relation, so each relation the node actually has gets exactly one
    // section (§4.4's "# Requires" / "# Contains" pattern).
    let mut by_relation: Vec<(Relation, Vec<&Link>)> = Vec::new();
    for link in node.links() {
        if let Some(entry) = by_relation.iter_mut().find(|(r, _)| *r == link.relation) {
            entry.1.push(link);
        } else {
            by_relation.push((link.relation, vec![link]));
        }
    }

    // One markdown link per target and concept (okf_validator flags a
    // second one as redundant): a target under two headings -- a child
    // the topic also references, a copy-to copy's source it also reuses
    // -- is linked under the first and named under the next.
    let mut linked: HashSet<&str> = HashSet::new();
    for (relation, links) in by_relation {
        body.push_str(&format!("# {}\n\n", relation.section_heading()));
        for link in links {
            let target = link.target.as_str();
            let (target_dir, target_title) =
                index.get(target).copied().unwrap_or(("topics", target));
            if linked.insert(target) {
                let href = relative_link(node.bundle_dir(), target_dir, target);
                body.push_str(&format!("- [{}]({href})", escape_link_syntax(target_title)));
            } else {
                body.push_str(&format!(
                    "- {} (`{target}`, linked above)",
                    escape_link_syntax(target_title)
                ));
            }
            // Which elements of the source are reused (generated-from):
            // plain text after the link, so okf_validator still sees one
            // ordinary markdown link per target.
            if !link.fragments.is_empty() {
                body.push_str(&format!(" (fragments: {})", link.fragments.join(", ")));
            }
            body.push('\n');
        }
        body.push('\n');
    }
    render_declarations(node, &mut body);

    Ok(format!("---\n{yaml}---\n\n{}", body.trim_end()))
}

/// The sections for what a node declares besides its links: the map's key
/// definitions, a topic's keys, external links and sample files. Keys and
/// files are plain text, not links: they have no concept file to point at
/// (okf_validator would flag a dangling link), and a key naming a topic
/// the map also contains would be a redundant one. URLs are markdown
/// links, which okf_validator leaves alone.
fn render_declarations(node: &NormalizedNode, body: &mut String) {
    match node {
        NormalizedNode::Map(map) if !map.key_defs.is_empty() => {
            body.push_str("# Key Definitions\n\n");
            for key in &map.key_defs {
                body.push_str(&format!("- `{}`", key.qualified_name()));
                if let Some(target) = &key.target {
                    body.push_str(&format!(" -> topic `{target}`"));
                } else if let Some(url) = &key.url {
                    body.push_str(&format!(" -> <{url}>"));
                } else if let Some(file) = &key.file {
                    body.push_str(&format!(" -> file `{file}`"));
                }
                if let Some(text) = &key.text {
                    body.push_str(&format!(" \"{}\"", escape_link_syntax(text)));
                }
                body.push_str(&format!(" (defined in `{}`)\n", key.source_file));
            }
            body.push('\n');
        }
        NormalizedNode::Topic(topic) => {
            if !topic.keyrefs.is_empty() {
                body.push_str("# Keys Used\n\n");
                for key in &topic.keyrefs {
                    body.push_str(&format!("- `{key}`\n"));
                }
                body.push('\n');
            }
            if !topic.external_links.is_empty() {
                body.push_str("# External Links\n\n");
                for link in &topic.external_links {
                    body.push_str(&format!("- <{}>", link.url));
                    if let Some(key) = &link.key {
                        body.push_str(&format!(" (key `{key}`)"));
                    }
                    body.push('\n');
                }
                body.push('\n');
            }
            if !topic.code_refs.is_empty() {
                body.push_str("# Code Files\n\n");
                for path in &topic.code_refs {
                    body.push_str(&format!("- `{path}`\n"));
                }
                body.push('\n');
            }
        }
        NormalizedNode::Map(_) => {}
    }
}

/// Plain text made safe to embed in markdown: a `](` the text happens to
/// contain -- a topic documenting markdown itself, with `[a link](topic.md)`
/// in an example -- would otherwise read as a link, which okf_validator
/// then rejects as dangling (dita-ot/docs' `Markdown-DITA-syntax`, once
/// nested topics' text was kept). Applied to every piece of free text a
/// concept or `index.md` carries: summary, content, titles, key text.
/// `]\(` renders the same text and is no link.
fn escape_link_syntax(text: &str) -> String {
    text.replace("](", "]\\(")
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
        body.push_str(&format!(
            "- [{}](maps/{}.md)\n",
            escape_link_syntax(node.title()),
            node.id()
        ));
    }
    body.push('\n');

    body.push_str("## Topics\n\n");
    for node in nodes
        .iter()
        .filter(|n| matches!(n, NormalizedNode::Topic(_)))
    {
        body.push_str(&format!(
            "- [{}](topics/{}.md)\n",
            escape_link_syntax(node.title()),
            node.id()
        ));
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

fn write_graph_json(output_dir: &Path, graph: &crate::graph_view::GraphView) -> Result<()> {
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
                key_defs: vec![],
                id: "user-guide".into(),
                title: "User Guide".into(),
                source_file: "user-guide.ditamap".into(),
                links: vec![
                    Link {
                        relation: Relation::Contains,
                        target: "installing-product".into(),
                        fragments: Vec::new(),
                    },
                    Link {
                        relation: Relation::Contains,
                        target: "configuration".into(),
                        fragments: Vec::new(),
                    },
                ],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                keyrefs: vec![],
                external_links: vec![],
                code_refs: vec![],
                id: "installing-product".into(),
                topic_type: TopicType::Task,
                title: "Installing Product".into(),
                shortdesc: Some("Steps to install the product in a production environment.".into()),
                body: Some(
                    "Download the installer package for your platform. Run the installer.".into(),
                ),
                audience: vec!["admin".into()],
                product: vec!["enterprise".into()],
                keys: vec!["install-task".into()],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                resolved_body: None,
                processing_role: None,
                dita_class: None,
                source_file: "topics/installing-product.dita".into(),
                links: vec![
                    Link {
                        relation: Relation::Requires,
                        target: "configuration".into(),
                        fragments: Vec::new(),
                    },
                    Link {
                        relation: Relation::Contains,
                        target: "installing-product-prereqs".into(),
                        fragments: Vec::new(),
                    },
                ],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                keyrefs: vec![],
                external_links: vec![],
                code_refs: vec![],
                id: "installing-product-prereqs".into(),
                topic_type: TopicType::Topic,
                title: "Installing Product: Prerequisites".into(),
                shortdesc: None,
                body: None,
                audience: vec!["admin".into()],
                product: vec!["enterprise".into()],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                resolved_body: None,
                processing_role: None,
                dita_class: None,
                source_file: "topics/installing-product-prereqs.dita".into(),
                links: vec![Link {
                    relation: Relation::References,
                    target: "configuration".into(),
                    fragments: Vec::new(),
                }],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                keyrefs: vec![],
                external_links: vec![],
                code_refs: vec![],
                id: "configuration".into(),
                topic_type: TopicType::Concept,
                title: "Configuration Overview".into(),
                shortdesc: None,
                body: Some("Configuration overview content goes here.".into()),
                audience: vec![],
                product: vec![],
                keys: vec!["config-concept".into()],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                resolved_body: None,
                processing_role: None,
                dita_class: None,
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

        let summary = write_bundle(&nodes, dir.path(), generated_at, true).unwrap();
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
    fn markdown_link_syntax_in_topic_text_is_not_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let mut nodes = sample_nodes();
        if let NormalizedNode::Topic(t) = &mut nodes[1] {
            t.body = Some("Write [a link](topic.md) to link a topic.".into());
        }
        // A key's text quoting a link, and a target's title: where they
        // are cited (the map's key list, the link text, index.md). The
        // title's own frontmatter keeps it verbatim -- it is data, and
        // okf_validator reads frontmatter too, so a title quoting a `.md`
        // link would still be flagged there.
        if let NormalizedNode::Topic(t) = &mut nodes[3] {
            t.title = "The [x](y) syntax".into();
        }
        if let NormalizedNode::Map(m) = &mut nodes[0] {
            m.key_defs = vec![crate::model::KeyDef {
                name: "md".into(),
                scope: None,
                target: None,
                url: None,
                file: None,
                text: Some("[a link](topic.md)".into()),
                source_file: "keys.ditamap".into(),
            }];
        }
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at, true).unwrap();

        let concept =
            fs::read_to_string(dir.path().join("okf/topics/installing-product.md")).unwrap();
        assert!(
            concept.contains("Write [a link]\\(topic.md) to link a topic."),
            "{concept}"
        );
        assert!(
            concept.contains("- [The [x]\\(y) syntax](configuration.md)"),
            "{concept}"
        );
        let map = fs::read_to_string(dir.path().join("okf/maps/user-guide.md")).unwrap();
        assert!(map.contains("`md` \"[a link]\\(topic.md)\""), "{map}");
        let index = fs::read_to_string(dir.path().join("okf/index.md")).unwrap();
        assert!(
            index.contains("- [The [x]\\(y) syntax](topics/configuration.md)"),
            "{index}"
        );
        let report = validate_bundle(&dir.path().join("okf")).unwrap();
        assert!(!report.has_errors(), "{:#?}", report.issues);
    }

    #[test]
    fn generated_from_names_its_fragments_after_the_link_and_in_graph_json() {
        let dir = tempfile::tempdir().unwrap();
        let mut nodes = sample_nodes();
        if let NormalizedNode::Topic(t) = &mut nodes[1] {
            t.links.push(Link {
                relation: Relation::GeneratedFrom,
                target: "configuration".into(),
                fragments: vec!["warning-note".into(), "li@37".into()],
            });
        }
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at, true).unwrap();

        let concept =
            fs::read_to_string(dir.path().join("okf/topics/installing-product.md")).unwrap();
        // The topic already links configuration under # Requires: one
        // markdown link per target, the second mention is named only.
        assert!(
            concept.contains(
                "# Generated From\n\n- Configuration Overview (`configuration`, linked above) (fragments: warning-note, li@37)"
            ),
            "{concept}"
        );
        assert_eq!(
            concept.matches("](configuration.md)").count(),
            1,
            "{concept}"
        );
        let graph: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("graph.json")).unwrap())
                .unwrap();
        let edge = graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["relation"] == "generated-from")
            .unwrap();
        assert_eq!(
            edge["fragments"],
            serde_json::json!(["warning-note", "li@37"])
        );
        // Every other edge stays as before: no empty "fragments" key.
        assert!(
            graph["edges"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["relation"] != "generated-from")
                .all(|e| e.get("fragments").is_none())
        );
        let report = validate_bundle(&dir.path().join("okf")).unwrap();
        assert!(!report.has_errors(), "{:#?}", report.issues);
        assert!(
            report
                .issues
                .iter()
                .all(|i| !i.message.contains("redundant link")),
            "{:#?}",
            report.issues
        );
    }

    #[test]
    fn task_frontmatter_carries_requires_and_contains_but_not_references() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at, true).unwrap();

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
    fn topic_body_renders_under_a_content_heading() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at, true).unwrap();

        let task = fs::read_to_string(dir.path().join("okf/topics/installing-product.md")).unwrap();
        assert!(task.contains("# Content\n\nDownload the installer package"));
        // "# Content" must come after "# Summary" (shortdesc), before the relation sections.
        assert!(task.find("# Summary").unwrap() < task.find("# Content").unwrap());
        assert!(task.find("# Content").unwrap() < task.find("# Requires").unwrap());

        let configuration =
            fs::read_to_string(dir.path().join("okf/topics/configuration.md")).unwrap();
        // No shortdesc on this topic, so no "# Summary" -- just "# Content".
        assert!(!configuration.contains("# Summary"));
        assert!(configuration.contains("# Content\n\nConfiguration overview content goes here."));
    }

    #[test]
    fn map_links_to_topics_across_directories() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at, true).unwrap();

        let map = fs::read_to_string(dir.path().join("okf/maps/user-guide.md")).unwrap();
        assert!(map.contains("../topics/installing-product.md"));
        assert!(map.contains("../topics/configuration.md"));
    }

    #[test]
    fn graph_json_is_a_flattened_view() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at, true).unwrap();

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

    #[test]
    fn emit_graph_json_false_skips_writing_it() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), generated_at, false).unwrap();

        assert!(!dir.path().join("graph.json").exists());
        // The okf/ bundle itself is unaffected -- emit_graph_json only
        // controls the derived, disposable graph.json (§2.3, §2.4).
        assert!(dir.path().join("okf/topics/configuration.md").exists());
    }

    /// Phase 6+'s "Incremental rebuild": a second `write_bundle` call
    /// with byte-for-byte identical nodes must not rewrite any concept
    /// file (proven by mtime, not just content -- content alone can't
    /// tell a skip from "rewrote it with the same bytes") and must
    /// report every one of them as `concepts_unchanged`, not
    /// `topics_written`/`maps_written`.
    #[test]
    fn rebuilding_with_identical_nodes_touches_no_concept_files() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let first_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        let first = write_bundle(&nodes, dir.path(), first_at, true).unwrap();
        assert_eq!(
            first.concepts_unchanged, 0,
            "nothing to skip on a first build"
        );

        let installing_path = dir.path().join("okf/topics/installing-product.md");
        let mtime_before = fs::metadata(&installing_path).unwrap().modified().unwrap();

        // A later generated_at, same nodes -- if the skip logic didn't
        // work, this alone would still change every file's `generated.at`.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let second_at: DateTime<Utc> = "2026-08-03T01:00:00Z".parse().unwrap();
        let second = write_bundle(&nodes, dir.path(), second_at, true).unwrap();

        assert_eq!(second.topics_written, 0);
        assert_eq!(second.maps_written, 0);
        assert_eq!(
            second.concepts_unchanged, 4,
            "all 4 nodes should be skipped"
        );

        let mtime_after = fs::metadata(&installing_path).unwrap().modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "an unchanged topic's concept file must not be rewritten"
        );
        let content = fs::read_to_string(&installing_path).unwrap();
        assert!(
            content.contains("2026-08-03T00:00:00"),
            "the file should still carry the *first* build's timestamp, not the second's: {content}"
        );
    }

    /// A node that *did* change must still be rewritten (with a fresh
    /// timestamp), while a node that neither changed nor links to
    /// anything that did stays untouched -- proves the skip is per-node,
    /// not all-or-nothing. `installing-product-prereqs` links to nothing
    /// that changes here and nothing links to *it*, so it's the control:
    /// `installing-product` (own title changed) and `user-guide` (links
    /// to it, so it must pick up the new title, per the cascading case
    /// covered separately below) are the two expected to be rewritten.
    #[test]
    fn rebuilding_after_one_node_changes_rewrites_only_that_node_and_its_referrers() {
        let dir = tempfile::tempdir().unwrap();
        let mut nodes = sample_nodes();
        let first_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), first_at, true).unwrap();

        let prereqs_path = dir.path().join("okf/topics/installing-product-prereqs.md");
        let prereqs_mtime_before = fs::metadata(&prereqs_path).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(1100));
        for node in &mut nodes {
            if let NormalizedNode::Topic(t) = node
                && t.id == "installing-product"
            {
                t.title = "Installing Product (Updated)".into();
            }
        }
        let second_at: DateTime<Utc> = "2026-08-03T01:00:00Z".parse().unwrap();
        let second = write_bundle(&nodes, dir.path(), second_at, true).unwrap();

        assert_eq!(
            second.topics_written, 1,
            "only installing-product's own fields changed"
        );
        assert_eq!(
            second.maps_written, 1,
            "user-guide contains installing-product, so it must pick up the new title"
        );
        assert_eq!(second.concepts_unchanged, 2);

        let prereqs_mtime_after = fs::metadata(&prereqs_path).unwrap().modified().unwrap();
        assert_eq!(
            prereqs_mtime_before, prereqs_mtime_after,
            "a topic unrelated to the change must not be rewritten"
        );

        let installing =
            fs::read_to_string(dir.path().join("okf/topics/installing-product.md")).unwrap();
        assert!(installing.contains("Installing Product (Updated)"));
        assert!(installing.contains("2026-08-03T01:00:00"));

        let user_guide = fs::read_to_string(dir.path().join("okf/maps/user-guide.md")).unwrap();
        assert!(
            user_guide.contains("Installing Product (Updated)"),
            "user-guide's own rendered link text must reflect installing-product's new title: {user_guide}"
        );
    }

    /// The correctness case `incremental.rs`'s own docs call out:
    /// `render_concept` inlines a link target's *title*, so a topic that
    /// links to a renamed one must be re-rendered even though its own
    /// fields never changed -- otherwise it would keep showing the old
    /// title forever, since nothing about the linking topic itself ever
    /// changes again to trigger a fresh render.
    #[test]
    fn rebuilding_after_a_link_targets_title_changes_rewrites_the_referencing_topic_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut nodes = sample_nodes();
        let first_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), first_at, true).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(1100));
        for node in &mut nodes {
            if let NormalizedNode::Topic(t) = node
                && t.id == "configuration"
            {
                t.title = "Configuration Overview (Renamed)".into();
            }
        }
        let second_at: DateTime<Utc> = "2026-08-03T01:00:00Z".parse().unwrap();
        write_bundle(&nodes, dir.path(), second_at, true).unwrap();

        // installing-product requires configuration (sample_nodes), so
        // its rendered "# Requires" section must now show the new title.
        let installing =
            fs::read_to_string(dir.path().join("okf/topics/installing-product.md")).unwrap();
        assert!(
            installing.contains("Configuration Overview (Renamed)"),
            "installing-product must be re-rendered to pick up configuration's new title: {installing}"
        );
    }
}
