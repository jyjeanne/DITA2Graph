//! RDF export of the graph (`docs/semantic-model.md`): the same normalized
//! model the OKF bundle comes from, written as Turtle to `rdf/graph.ttl`,
//! with the `d2g` ontology and its SHACL shapes beside it
//! (`rdf/ontology.ttl`, `rdf/shapes.ttl`).
//!
//! Nothing is inferred at build time, so the export writes itself what the
//! shapes and SPARQL queries need: every node's parent types, both
//! directions of `d2g:contains` and `d2g:requires` (a prerequisite is also
//! a `d2g:references`), and the qualification of each `d2g:generatedFrom`
//! (which fragments of which source). The map's keys, the topics' external
//! links and included sample files are nodes of their own. Output is
//! sorted, so the same model and build time always give the same file.

use crate::model::{KeyDef, Link, NormalizedNode, NormalizedTopic, Relation, TopicType};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// Namespace of the `d2g` vocabulary.
pub const NAMESPACE: &str = "https://w3id.org/dita2graph/ns#";
/// The `d2g` ontology (OWL 2 RL), copied to `rdf/ontology.ttl`.
pub const ONTOLOGY_TTL: &str = include_str!("../ontology/d2g.ttl");
/// The SHACL shapes checking an export, copied to `rdf/shapes.ttl`.
pub const SHAPES_TTL: &str = include_str!("../ontology/d2g-shapes.ttl");
/// The files `write_rdf` writes under `rdf/`: the data, the ontology and
/// the shapes. A build without the export removes exactly these.
pub const RDF_FILES: [&str; 3] = ["graph.ttl", "ontology.ttl", "shapes.ttl"];

/// Base used when none is given: the reserved `.invalid` domain, so these
/// IRIs can never collide with real ones.
const PLACEHOLDER_ROOT: &str = "https://dita2graph.invalid/";

/// The prefixes the export declares; the MCP server's `query_graph`
/// predeclares the same ones.
pub const PREFIXES: &[(&str, &str)] = &[
    ("d2g", NAMESPACE),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RdfSummary {
    pub triples: usize,
    /// The base the data IRIs were built on.
    pub base: String,
    /// Whether that base is the `.invalid` placeholder (no `--rdf-base`).
    pub placeholder_base: bool,
}

/// Writes `rdf/graph.ttl`, `rdf/ontology.ttl` and `rdf/shapes.ttl` under
/// `output`. `base` is `--rdf-base`; without it, the data IRIs live under
/// `https://dita2graph.invalid/<map id>/`.
pub fn write_rdf(
    nodes: &[NormalizedNode],
    output: &Path,
    base: Option<&str>,
    generator: &str,
    generated_at: DateTime<Utc>,
) -> Result<RdfSummary> {
    let (base, placeholder_base) = resolve_base(base, nodes)?;
    let graph = build_graph(nodes, &base, generator, generated_at);
    let dir = output.join("rdf");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let [graph_file, ontology_file, shapes_file] = RDF_FILES;
    for (file, content) in [
        (graph_file, graph.to_turtle()),
        (ontology_file, ONTOLOGY_TTL.to_string()),
        (shapes_file, SHAPES_TTL.to_string()),
    ] {
        fs::write(dir.join(file), content)
            .with_context(|| format!("writing {}", dir.join(file).display()))?;
    }
    Ok(RdfSummary {
        triples: graph.len(),
        base,
        placeholder_base,
    })
}

/// Checks that `base` is an absolute IRI (a scheme, then something, and no
/// character Turtle forbids inside `<...>`) and returns it ending with `/`
/// or `#`, ready to have paths appended.
pub fn check_base(base: &str) -> Result<String> {
    let scheme_ok = base.split_once(':').is_some_and(|(scheme, rest)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && !rest.is_empty()
    });
    if !scheme_ok
        || base
            .chars()
            .any(|c| " <>\"{}|^`\\".contains(c) || c.is_control())
    {
        bail!("--rdf-base: {base:?} is not an absolute IRI");
    }
    let mut base = base.to_string();
    if !base.ends_with('/') && !base.ends_with('#') {
        base.push('/');
    }
    Ok(base)
}

/// The data IRIs' base, ending with `/` or `#`, and whether it is the
/// placeholder.
fn resolve_base(base: Option<&str>, nodes: &[NormalizedNode]) -> Result<(String, bool)> {
    match base {
        Some(base) => Ok((check_base(base)?, false)),
        None => {
            let map_id = nodes
                .iter()
                .find_map(|n| match n {
                    NormalizedNode::Map(m) => Some(m.id.as_str()),
                    NormalizedNode::Topic(_) => None,
                })
                .unwrap_or("graph");
            Ok((
                format!("{PLACEHOLDER_ROOT}{}/", encode_segment(map_id)),
                true,
            ))
        }
    }
}

/// Subject -> predicate -> objects, every term already written as Turtle.
/// Sorted maps and sets keep the output stable.
#[derive(Default)]
struct Graph {
    subjects: BTreeMap<String, BTreeMap<&'static str, BTreeSet<String>>>,
}

impl Graph {
    fn add(&mut self, subject: &str, predicate: &'static str, object: String) {
        self.subjects
            .entry(subject.to_string())
            .or_default()
            .entry(predicate)
            .or_default()
            .insert(object);
    }

    fn len(&self) -> usize {
        self.subjects
            .values()
            .flat_map(|predicates| predicates.values())
            .map(BTreeSet::len)
            .sum()
    }

    fn to_turtle(&self) -> String {
        let mut out = String::from(
            "# Generated by dita2graph-core from the normalized DITA model.\n\
             # Vocabulary: ontology.ttl; validity rules: shapes.ttl.\n",
        );
        for (prefix, iri) in PREFIXES {
            let _ = writeln!(out, "@prefix {prefix}: <{iri}> .");
        }
        for (subject, predicates) in &self.subjects {
            let _ = write!(out, "\n{subject}");
            let mut first = true;
            // "a" sorts before every prefixed predicate, so the types come first.
            for (predicate, objects) in predicates {
                let objects: Vec<&str> = objects.iter().map(String::as_str).collect();
                let _ = write!(
                    out,
                    "{}\n    {predicate} {}",
                    if first { "" } else { " ;" },
                    objects.join(" , ")
                );
                first = false;
            }
            out.push_str(" .\n");
        }
        out
    }
}

/// Builds the graph of `nodes` (see the module docs) on `base`.
fn build_graph(
    nodes: &[NormalizedNode],
    base: &str,
    generator: &str,
    generated_at: DateTime<Utc>,
) -> Graph {
    let mut graph = Graph::default();
    let build = iri(&format!(
        "{base}build/{}",
        generated_at
            .to_rfc3339_opts(SecondsFormat::Secs, true)
            .replace(':', "-")
    ));
    graph.add(&build, "a", "prov:Activity".into());
    graph.add(
        &build,
        "prov:endedAtTime",
        format!(
            "\"{}\"^^xsd:dateTime",
            generated_at.to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
    );
    graph.add(&build, "d2g:generator", literal(generator));

    // Link targets name node ids; the map and the topics live under
    // different paths, so resolve them through one table.
    let node_iris: HashMap<&str, String> = nodes
        .iter()
        .map(|node| (node.id(), node_iri(base, node)))
        .collect();
    let target_iri = |id: &str| {
        node_iris
            .get(id)
            .cloned()
            .unwrap_or_else(|| iri(&format!("{base}node/{}", encode_segment(id))))
    };
    // A topic names a key the way its @keyref does: `scope.name`.
    let key_iris: HashMap<String, String> = nodes
        .iter()
        .filter_map(|node| match node {
            NormalizedNode::Map(m) => Some(&m.key_defs),
            NormalizedNode::Topic(_) => None,
        })
        .flatten()
        .map(|key| {
            (
                key.qualified_name(),
                key_iri(base, key.scope.as_deref(), &key.name),
            )
        })
        .collect();

    for node in nodes {
        let subject = node_iri(base, node);
        for class in node_classes(node) {
            graph.add(&subject, "a", (*class).to_string());
        }
        graph.add(&subject, "dcterms:identifier", literal(node.id()));
        graph.add(&subject, "dcterms:title", literal(node.title()));
        graph.add(
            &subject,
            "d2g:okfConcept",
            literal(&format!("okf/{}/{}.md", node.bundle_dir(), node.id())),
        );
        graph.add(&subject, "prov:wasGeneratedBy", build.clone());

        // A heading's source file is "<map>#<heading id>"; its file is the map.
        let path = node.source_file().split('#').next().unwrap_or_default();
        let file = add_file(&mut graph, base, path);
        graph.add(&subject, "d2g:sourceFile", file);

        match node {
            NormalizedNode::Topic(topic) => {
                add_topic_metadata(&mut graph, &subject, topic);
                add_topic_declarations(&mut graph, base, &subject, topic, &key_iris);
            }
            NormalizedNode::Map(map) => {
                for key in &map.key_defs {
                    add_key(&mut graph, base, &subject, key, &target_iri);
                }
            }
        }
        for link in node.links() {
            add_link(&mut graph, base, node.id(), &subject, link, &target_iri);
        }
    }
    graph
}

/// A source file's node (`file/<path>`), typed and with its path; returns its IRI.
fn add_file(graph: &mut Graph, base: &str, path: &str) -> String {
    let file = iri(&format!("{base}file/{}", encode_path(path)));
    graph.add(&file, "a", "d2g:SourceFile".into());
    graph.add(&file, "d2g:path", literal(path));
    file
}

/// `key/<scope>/<name>`, `_` standing for the root scope.
fn key_iri(base: &str, scope: Option<&str>, name: &str) -> String {
    iri(&format!(
        "{base}key/{}/{}",
        scope.map_or_else(|| "_".to_string(), encode_segment),
        encode_segment(name)
    ))
}

/// A URL as an IRI: the characters Turtle forbids between `<` and `>`
/// (spaces, quotes, braces...) are percent-encoded; the rest is kept.
fn url_iri(url: &str) -> String {
    let mut out = String::with_capacity(url.len() + 2);
    out.push('<');
    for c in url.chars() {
        if " <>\"{}|^`\\".contains(c) || c.is_control() {
            let mut buf = [0; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{byte:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out.push('>');
    out
}

/// One key definition of `map`: the key, its name, the map and file that
/// define it, and what it names (a topic, a URL, a file) or its text.
fn add_key(
    graph: &mut Graph,
    base: &str,
    map: &str,
    key: &KeyDef,
    target_iri: &impl Fn(&str) -> String,
) {
    let subject = key_iri(base, key.scope.as_deref(), &key.name);
    graph.add(&subject, "a", "d2g:Key".into());
    graph.add(&subject, "d2g:keyName", literal(&key.name));
    graph.add(map, "d2g:definesKey", subject.clone());
    graph.add(&subject, "d2g:definedIn", map.to_string());
    let file = add_file(graph, base, &key.source_file);
    graph.add(&subject, "d2g:sourceFile", file);
    if let Some(target) = &key.target {
        graph.add(&subject, "d2g:keyTarget", target_iri(target));
    } else if let Some(url) = &key.url {
        let resource = url_iri(url);
        graph.add(&resource, "a", "d2g:ExternalResource".into());
        graph.add(&subject, "d2g:keyTarget", resource);
    } else if let Some(path) = &key.file {
        let file = add_file(graph, base, path);
        graph.add(&subject, "d2g:keyTarget", file);
    }
    if let Some(text) = key.text.as_deref().filter(|t| !t.is_empty()) {
        graph.add(&subject, "d2g:keyText", literal(text));
    }
}

/// What the topic declares besides its links: the keys it uses (a key no
/// map defines is still a node, so the shapes can report it), the URLs it
/// links to and the sample files it includes.
fn add_topic_declarations(
    graph: &mut Graph,
    base: &str,
    subject: &str,
    topic: &NormalizedTopic,
    key_iris: &HashMap<String, String>,
) {
    for name in &topic.keyrefs {
        let key = match key_iris.get(name) {
            Some(key) => key.clone(),
            None => {
                let key = key_iri(base, None, name);
                graph.add(&key, "a", "d2g:Key".into());
                graph.add(&key, "d2g:keyName", literal(name));
                key
            }
        };
        graph.add(subject, "d2g:usesKey", key);
    }
    for link in &topic.external_links {
        let resource = url_iri(&link.url);
        graph.add(&resource, "a", "d2g:ExternalResource".into());
        graph.add(subject, "d2g:linksTo", resource);
    }
    for path in &topic.code_refs {
        let file = add_file(graph, base, path);
        graph.add(subject, "d2g:includesCode", file);
    }
}

fn node_iri(base: &str, node: &NormalizedNode) -> String {
    match node {
        NormalizedNode::Map(m) => iri(&format!("{base}map/{}", encode_segment(&m.id))),
        NormalizedNode::Topic(t) => iri(&format!("{base}node/{}", encode_segment(&t.id))),
    }
}

/// The node's class and every class above it in the ontology.
fn node_classes(node: &NormalizedNode) -> &'static [&'static str] {
    match node {
        NormalizedNode::Map(_) => &["d2g:Map", "d2g:Node"],
        NormalizedNode::Topic(t) => match t.topic_type {
            TopicType::Concept => &["d2g:Concept", "d2g:Topic", "d2g:Node"],
            TopicType::Task => &["d2g:Task", "d2g:Topic", "d2g:Node"],
            TopicType::Reference => &["d2g:Reference", "d2g:Topic", "d2g:Node"],
            TopicType::Glossentry => &["d2g:GlossEntry", "skos:Concept", "d2g:Topic", "d2g:Node"],
            TopicType::Troubleshooting => &["d2g:Troubleshooting", "d2g:Topic", "d2g:Node"],
            TopicType::Topic => &["d2g:Topic", "d2g:Node"],
            TopicType::Topichead => &["d2g:TopicHead", "d2g:Node"],
        },
    }
}

fn add_topic_metadata(graph: &mut Graph, subject: &str, topic: &NormalizedTopic) {
    if let Some(shortdesc) = topic.shortdesc.as_deref().filter(|s| !s.is_empty()) {
        graph.add(subject, "dcterms:description", literal(shortdesc));
    }
    for audience in &topic.audience {
        graph.add(subject, "d2g:audience", literal(audience));
    }
    for product in &topic.product {
        graph.add(subject, "d2g:product", literal(product));
    }
    if let Some(role) = &topic.processing_role {
        graph.add(subject, "d2g:processingRole", literal(role));
    }
    if let Some(class) = &topic.dita_class {
        graph.add(subject, "d2g:ditaClass", literal(class));
    }
}

fn add_link(
    graph: &mut Graph,
    base: &str,
    source_id: &str,
    subject: &str,
    link: &Link,
    target_iri: &impl Fn(&str) -> String,
) {
    let target = target_iri(&link.target);
    match link.relation {
        Relation::Contains => {
            graph.add(subject, "d2g:contains", target.clone());
            graph.add(&target, "d2g:containedIn", subject.to_string());
        }
        Relation::References => graph.add(subject, "d2g:references", target),
        // A prerequisite (a task's prereq link, or its link marked
        // importance="required") is also a reference, and its inverse.
        Relation::Requires => {
            graph.add(subject, "d2g:requires", target.clone());
            graph.add(subject, "d2g:references", target.clone());
            graph.add(&target, "d2g:prerequisiteFor", subject.to_string());
        }
        Relation::CopyOf => graph.add(subject, "d2g:copyOf", target),
        Relation::RelatedTo => graph.add(subject, "d2g:relatedTo", target),
        Relation::AppliesTo => graph.add(subject, "d2g:appliesTo", target),
        Relation::GeneratedFrom => {
            graph.add(subject, "d2g:generatedFrom", target.clone());
            let reuse = iri(&format!(
                "{base}node/{}/reuse/{}",
                encode_segment(source_id),
                encode_segment(&link.target)
            ));
            graph.add(subject, "d2g:qualifiedReuse", reuse.clone());
            graph.add(&reuse, "a", "d2g:Reuse".into());
            graph.add(&reuse, "d2g:reuseSource", target.clone());
            for fragment in &link.fragments {
                let fragment_iri = iri(&format!(
                    "{base}node/{}/fragment/{}",
                    encode_segment(&link.target),
                    encode_segment(fragment)
                ));
                graph.add(&reuse, "d2g:reusedFragment", fragment_iri.clone());
                graph.add(&fragment_iri, "a", "d2g:Fragment".into());
                graph.add(&fragment_iri, "dcterms:identifier", literal(fragment));
                graph.add(&fragment_iri, "d2g:fragmentOf", target.clone());
            }
        }
    }
}

fn iri(full: &str) -> String {
    format!("<{full}>")
}

/// A Turtle string literal (`"..."`), escaped.
fn literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Percent-encodes everything but the unreserved characters of RFC 3986,
/// so any id fits in one IRI path segment.
fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// [`encode_segment`] on each `/`-separated segment of a relative path.
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NormalizedMap, NormalizedTopic};
    use chrono::TimeZone;

    fn topic(id: &str, topic_type: TopicType, links: Vec<Link>) -> NormalizedNode {
        NormalizedNode::Topic(NormalizedTopic {
            keyrefs: vec![],
            external_links: vec![],
            code_refs: vec![],
            id: id.into(),
            topic_type,
            title: format!("Title of {id}"),
            shortdesc: None,
            body: None,
            resolved_body: None,
            processing_role: None,
            dita_class: None,
            audience: vec![],
            product: vec![],
            keys: vec![],
            uicontrols: vec![],
            cmd_uicontrols: vec![],
            authored_file: None,
            source_file: format!("topics/{id}.dita"),
            links,
        })
    }

    fn link(relation: Relation, target: &str) -> Link {
        Link {
            relation,
            target: target.into(),
            fragments: vec![],
        }
    }

    fn sample() -> Vec<NormalizedNode> {
        vec![
            NormalizedNode::Map(NormalizedMap {
                key_defs: vec![],
                id: "user-guide".into(),
                title: "User Guide".into(),
                source_file: "user-guide.ditamap".into(),
                links: vec![link(Relation::Contains, "install")],
            }),
            topic(
                "install",
                TopicType::Task,
                vec![
                    link(Relation::Requires, "config"),
                    Link {
                        relation: Relation::GeneratedFrom,
                        target: "warehouse".into(),
                        fragments: vec!["plug-in".into(), "li@37".into()],
                    },
                ],
            ),
            topic("config", TopicType::Concept, vec![]),
            topic("warehouse", TopicType::Topic, vec![]),
        ]
    }

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 10, 9, 30, 0).unwrap()
    }

    fn turtle(nodes: &[NormalizedNode]) -> String {
        build_graph(
            nodes,
            "https://example.org/ug/",
            "dita2graph-core/test",
            at(),
        )
        .to_turtle()
    }

    #[test]
    fn nodes_carry_their_parent_types_and_metadata() {
        let ttl = turtle(&sample());
        assert!(ttl.contains(
            "<https://example.org/ug/node/install>\n    a d2g:Node , d2g:Task , d2g:Topic ;"
        ));
        assert!(
            ttl.contains("<https://example.org/ug/map/user-guide>\n    a d2g:Map , d2g:Node ;")
        );
        assert!(ttl.contains("dcterms:title \"Title of install\""));
        assert!(ttl.contains("d2g:okfConcept \"okf/topics/install.md\""));
        assert!(ttl.contains("d2g:sourceFile <https://example.org/ug/file/topics/install.dita>"));
        assert!(ttl.contains("d2g:path \"topics/install.dita\""));
    }

    #[test]
    fn contains_is_written_both_ways() {
        let ttl = turtle(&sample());
        assert!(ttl.contains("d2g:contains <https://example.org/ug/node/install>"));
        assert!(ttl.contains("d2g:containedIn <https://example.org/ug/map/user-guide>"));
    }

    #[test]
    fn a_prerequisite_is_also_a_reference_and_written_both_ways() {
        let ttl = turtle(&sample());
        assert!(ttl.contains("d2g:requires <https://example.org/ug/node/config>"));
        assert!(ttl.contains("d2g:references <https://example.org/ug/node/config>"));
        assert!(ttl.contains("d2g:prerequisiteFor <https://example.org/ug/node/install>"));
    }

    #[test]
    fn keys_external_links_code_files_and_copies_are_exported() {
        let mut nodes = sample();
        if let NormalizedNode::Map(map) = &mut nodes[0] {
            let key = |name: &str| KeyDef {
                name: name.into(),
                scope: None,
                target: None,
                url: None,
                file: None,
                text: None,
                source_file: "keys.ditamap".into(),
            };
            map.key_defs = vec![
                KeyDef {
                    target: Some("config".into()),
                    ..key("config")
                },
                KeyDef {
                    url: Some("https://brew.sh/a b".into()),
                    text: Some("Homebrew".into()),
                    ..key("brew")
                },
                KeyDef {
                    scope: Some("legacy".into()),
                    text: Some("Acme".into()),
                    ..key("product")
                },
            ];
        }
        if let NormalizedNode::Topic(topic) = &mut nodes[1] {
            topic.keyrefs = vec!["brew".into(), "legacy.product".into(), "nowhere".into()];
            topic.external_links = vec![crate::model::ExternalLink {
                url: "https://brew.sh/a b".into(),
                key: Some("brew".into()),
            }];
            topic.code_refs = vec!["samples/build.xml".into()];
            topic.links.push(link(Relation::CopyOf, "config"));
        }
        let ttl = turtle(&nodes);
        let brew = "<https://example.org/ug/key/_/brew>";
        assert!(ttl.contains(&format!("{brew}\n    a d2g:Key ;")));
        assert!(ttl.contains(&format!("d2g:definesKey {brew} , <https://example.org/ug/key/_/config> , <https://example.org/ug/key/legacy/product>")));
        assert!(ttl.contains("d2g:definedIn <https://example.org/ug/map/user-guide>"));
        assert!(ttl.contains("d2g:keyTarget <https://brew.sh/a%20b>"));
        assert!(ttl.contains("d2g:keyText \"Homebrew\""));
        assert!(ttl.contains("d2g:sourceFile <https://example.org/ug/file/keys.ditamap>"));
        // A scoped key, named scope.key by the topic.
        assert!(ttl.contains("d2g:usesKey <https://example.org/ug/key/_/brew> , <https://example.org/ug/key/_/nowhere> , <https://example.org/ug/key/legacy/product>"));
        // A key no map defines is a key without definedIn, for the shapes to report.
        assert!(ttl.contains(
            "<https://example.org/ug/key/_/nowhere>\n    a d2g:Key ;\n    d2g:keyName \"nowhere\" ."
        ));
        assert!(ttl.contains("<https://brew.sh/a%20b>\n    a d2g:ExternalResource ."));
        assert!(ttl.contains("d2g:linksTo <https://brew.sh/a%20b>"));
        assert!(ttl.contains("d2g:includesCode <https://example.org/ug/file/samples/build.xml>"));
        assert!(ttl.contains("d2g:copyOf <https://example.org/ug/node/config>"));
        oxttl::TurtleParser::new()
            .for_slice(ttl.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .expect("valid Turtle");
    }

    #[test]
    fn a_troubleshooting_topic_is_typed_as_such() {
        let ttl = turtle(&[topic("fix", TopicType::Troubleshooting, vec![])]);
        assert!(ttl.contains("a d2g:Node , d2g:Topic , d2g:Troubleshooting ;"));
    }

    #[test]
    fn generated_from_is_qualified_by_its_fragments() {
        let ttl = turtle(&sample());
        let reuse = "<https://example.org/ug/node/install/reuse/warehouse>";
        assert!(ttl.contains(&format!("d2g:qualifiedReuse {reuse}")));
        assert!(ttl.contains(&format!("{reuse}\n    a d2g:Reuse ;")));
        // li@37 has no @id; '@' is percent-encoded in the IRI.
        assert!(ttl.contains("<https://example.org/ug/node/warehouse/fragment/li%4037>"));
        assert!(ttl.contains("d2g:fragmentOf <https://example.org/ug/node/warehouse>"));
        assert!(ttl.contains("dcterms:identifier \"li@37\""));
    }

    #[test]
    fn the_build_is_a_dated_activity() {
        let ttl = turtle(&sample());
        let build = "<https://example.org/ug/build/2026-11-10T09-30-00Z>";
        assert!(ttl.contains(&format!("{build}\n    a prov:Activity ;")));
        assert!(ttl.contains("prov:endedAtTime \"2026-11-10T09:30:00Z\"^^xsd:dateTime"));
        assert!(ttl.contains(&format!("prov:wasGeneratedBy {build}")));
    }

    #[test]
    fn output_is_stable_whatever_the_node_order() {
        let mut reversed = sample();
        reversed.reverse();
        assert_eq!(turtle(&sample()), turtle(&reversed));
    }

    #[test]
    fn literals_and_iris_are_escaped() {
        assert_eq!(literal("say \"hi\"\\\n"), "\"say \\\"hi\\\"\\\\\\n\"");
        assert_eq!(encode_segment("a b/é"), "a%20b%2F%C3%A9");
        assert_eq!(encode_path("topics/a b.dita"), "topics/a%20b.dita");
    }

    #[test]
    fn the_base_defaults_to_the_invalid_domain_and_is_checked() {
        let nodes = sample();
        assert_eq!(
            resolve_base(None, &nodes).unwrap(),
            ("https://dita2graph.invalid/user-guide/".to_string(), true)
        );
        assert_eq!(
            resolve_base(Some("https://docs.example.com/ug"), &nodes).unwrap(),
            ("https://docs.example.com/ug/".to_string(), false)
        );
        assert!(resolve_base(Some("not an iri"), &nodes).is_err());
        assert!(resolve_base(Some("relative/path"), &nodes).is_err());
    }

    #[test]
    fn a_heading_points_at_its_map_file() {
        let mut heading = topic("topichead--group", TopicType::Topichead, vec![]);
        if let NormalizedNode::Topic(t) = &mut heading {
            t.source_file = "user-guide.ditamap#topichead--group".into();
        }
        let ttl = turtle(&[heading]);
        assert!(ttl.contains("a d2g:Node , d2g:TopicHead ;"));
        assert!(ttl.contains("d2g:sourceFile <https://example.org/ug/file/user-guide.ditamap>"));
    }

    #[test]
    fn the_export_parses_as_turtle() {
        let ttl = turtle(&sample());
        let triples = oxttl::TurtleParser::new()
            .for_slice(ttl.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .expect("valid Turtle");
        let graph = build_graph(&sample(), "https://example.org/ug/", "t", at());
        assert_eq!(triples.len(), graph.len());
    }

    #[test]
    fn write_rdf_writes_the_graph_the_ontology_and_the_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let summary = write_rdf(&sample(), dir.path(), None, "t", at()).unwrap();
        assert!(summary.placeholder_base);
        assert!(summary.triples > 0);
        for file in ["graph.ttl", "ontology.ttl", "shapes.ttl"] {
            assert!(dir.path().join("rdf").join(file).is_file(), "{file}");
        }
    }
}
