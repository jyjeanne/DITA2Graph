//! The Java→Rust boundary: `tests/data/java-model.json` is the normalized
//! model as the DITA-OT plugin writes it, every field of every node kind
//! set, and the plugin's `ModelJsonContractTest` checks its serializer
//! still writes exactly that. Here the Rust model has to read all of it:
//! read and written back, the file must come out the same, so a field the
//! two sides name differently, or one the Rust model drops, fails.

use chrono::{TimeZone, Utc};
use dita2graph_core::{NormalizedNode, Relation, TopicType, graph_view, write_bundle};
use serde_json::Value;
use std::fs;
use std::path::Path;

fn java_model() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/java-model.json"))
        .unwrap()
}

#[test]
fn every_field_the_plugin_writes_is_read_back() {
    let raw = java_model();
    let nodes: Vec<NormalizedNode> = serde_json::from_str(&raw).unwrap();

    let written = serde_json::to_value(&nodes).unwrap();
    let read: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(written, read);
}

#[test]
fn the_values_land_in_the_right_fields() {
    let nodes: Vec<NormalizedNode> = serde_json::from_str(&java_model()).unwrap();

    let NormalizedNode::Map(map) = &nodes[0] else {
        panic!("the first node is the map")
    };
    let keys: Vec<_> = map.key_defs.iter().map(|k| k.qualified_name()).collect();
    assert_eq!(keys, ["install", "tools.brew", "logo", "product-name"]);
    assert_eq!(map.key_defs[2].file.as_deref(), Some("images/logo.png"));

    let NormalizedNode::Topic(task) = &nodes[1] else {
        panic!("the second node is the task")
    };
    assert_eq!(task.topic_type, TopicType::Task);
    assert_eq!(task.title, "Installing \"Widget\" — café");
    assert_eq!(
        task.body.as_deref(),
        Some("Run the installer.\tThen restart.\nDone.")
    );
    assert_eq!(task.cmd_uicontrols, ["Save"]);
    assert_eq!(task.external_links[1].key, None);
    assert_eq!(
        task.authored_file.as_deref(),
        Some("topics/install-source.dita")
    );
    assert_eq!(task.metadata.authors[1].type_, None);
    assert_eq!(
        task.metadata.audiences[0].experience_level.as_deref(),
        Some("expert")
    );
    assert_eq!(task.metadata.index_terms, [["installing", "on Linux"]]);
    let reuse = &task.links[2];
    assert_eq!(reuse.relation, Relation::GeneratedFrom);
    assert_eq!(reuse.fragments, ["warning", "li@37"]);
    assert_eq!(task.links[3].relation, Relation::CopyOf);

    let NormalizedNode::Topic(heading) = &nodes[4] else {
        panic!("the fifth node is the heading")
    };
    assert_eq!(heading.topic_type, TopicType::Topichead);
}

#[test]
fn the_model_builds_a_bundle_and_a_graph() {
    let nodes: Vec<NormalizedNode> = serde_json::from_str(&java_model()).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let summary = write_bundle(&nodes, dir.path(), at, true).unwrap();
    assert_eq!((summary.topics_written, summary.maps_written), (6, 1));

    let graph = graph_view(&nodes);
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.from == "installing")
        .map(|e| (e.relation.as_str(), e.to.as_str()))
        .collect();
    for edge in [
        ("requires", "prerequisites"),
        ("references", "settings"),
        ("generated-from", "shared"),
        ("copy-of", "install-source"),
    ] {
        assert!(edges.contains(&edge), "{edge:?} not in {edges:?}");
    }
}
