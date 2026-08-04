//! Writes the RAG-oriented content index alongside the OKF graph
//! (`docs/plugin-specification.md` §13.1): `rag/chunks.jsonl` (one
//! enriched record per topic) and `rag/metadata.json` (bundle-level
//! metadata), both derived from the same normalized-model slice
//! `okf::write_bundle` renders -- not a second parse of the DITA
//! source, per §13.1's "single pass, two correlated outputs".
//!
//! This is the first implemented piece of §13.1's design direction: the
//! `chunks.jsonl` records exist so a future content-search layer has
//! something to search, but no search/embedding/ranking logic lives
//! here, and no MCP tool consumes this file yet -- §13.1's query-routing
//! and `analyze_impact` pieces remain unimplemented. Maps aren't
//! chunked: a ditamap has no body prose of its own, only `contains`
//! relations already in the OKF graph (§4.4).

use crate::model::NormalizedNode;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Serialize)]
struct Chunk<'a> {
    id: &'a str,
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(rename = "topicType")]
    topic_type: &'static str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    audience: &'a [String],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    product: &'a [String],
    /// The identity that joins this chunk back to its OKF concept
    /// (§13.1), relative to `okf/` the same way `resource` in the
    /// concept's own frontmatter is relative to the DITA source (§4.4).
    #[serde(rename = "okfNode")]
    okf_node: String,
}

#[derive(Serialize)]
struct Metadata {
    generated: GeneratedBy,
    #[serde(rename = "chunkCount")]
    chunk_count: usize,
}

#[derive(Serialize)]
struct GeneratedBy {
    by: String,
    at: DateTime<Utc>,
}

/// A written RAG index's summary, for CLI reporting (`dita2graph-core
/// build`, §3.4).
#[derive(Debug, Default)]
pub struct RagSummary {
    pub chunks_written: usize,
}

/// Writes `<output_dir>/rag/chunks.jsonl` and
/// `<output_dir>/rag/metadata.json` from the same `nodes` slice
/// `okf::write_bundle` renders.
pub fn write_rag_index(
    nodes: &[NormalizedNode],
    output_dir: &Path,
    generated_at: DateTime<Utc>,
) -> Result<RagSummary> {
    let rag_dir = output_dir.join("rag");
    fs::create_dir_all(&rag_dir).context("creating rag/")?;

    let mut lines = String::new();
    let mut summary = RagSummary::default();
    for node in nodes {
        let NormalizedNode::Topic(topic) = node else {
            continue;
        };
        let text = chunk_text(topic.shortdesc.as_deref(), topic.body.as_deref());
        let chunk = Chunk {
            id: &topic.id,
            title: &topic.title,
            text,
            topic_type: topic.topic_type.okf_type(),
            audience: &topic.audience,
            product: &topic.product,
            okf_node: format!("topics/{}.md", topic.id),
        };
        lines.push_str(&serde_json::to_string(&chunk).context("serializing chunk")?);
        lines.push('\n');
        summary.chunks_written += 1;
    }
    fs::write(rag_dir.join("chunks.jsonl"), lines).context("writing rag/chunks.jsonl")?;

    let metadata = Metadata {
        generated: GeneratedBy {
            by: crate::okf::producer(),
            at: generated_at,
        },
        chunk_count: summary.chunks_written,
    };
    let metadata_json =
        serde_json::to_string_pretty(&metadata).context("serializing rag/metadata.json")?;
    fs::write(rag_dir.join("metadata.json"), metadata_json + "\n")
        .context("writing rag/metadata.json")?;

    Ok(summary)
}

/// Combines `shortdesc` and `body` into one searchable text field,
/// keeping both when present rather than picking one -- a topic's
/// shortdesc is often a one-line abstract that doesn't repeat in its
/// body (§4.4), so dropping either loses real content.
fn chunk_text(shortdesc: Option<&str>, body: Option<&str>) -> Option<String> {
    match (shortdesc, body) {
        (Some(shortdesc), Some(body)) => Some(format!("{shortdesc}\n\n{body}")),
        (Some(shortdesc), None) => Some(shortdesc.to_string()),
        (None, Some(body)) => Some(body.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Link, NormalizedMap, NormalizedTopic, Relation, TopicType};

    fn sample_nodes() -> Vec<NormalizedNode> {
        vec![
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
                body: Some("Download the installer. Run it.".into()),
                audience: vec!["admin".into()],
                product: vec!["enterprise".into()],
                keys: vec!["install-task".into()],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/installing-product.dita".into(),
                links: vec![],
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
        ]
    }

    #[test]
    fn writes_one_chunk_per_topic_and_skips_maps() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();

        let summary = write_rag_index(&nodes, dir.path(), generated_at).unwrap();
        assert_eq!(summary.chunks_written, 2, "2 topics, 1 map (not chunked)");

        let raw = fs::read_to_string(dir.path().join("rag/chunks.jsonl")).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            !raw.contains("user-guide"),
            "the map should not appear as a chunk"
        );
    }

    #[test]
    fn chunk_text_combines_shortdesc_and_body_when_both_present() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_rag_index(&nodes, dir.path(), generated_at).unwrap();

        let raw = fs::read_to_string(dir.path().join("rag/chunks.jsonl")).unwrap();
        let installing: serde_json::Value = raw
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .find(|v: &serde_json::Value| v["id"] == "installing-product")
            .unwrap();
        assert_eq!(
            installing["text"],
            "Steps to install the product.\n\nDownload the installer. Run it."
        );
        assert_eq!(installing["topicType"], "Task");
        assert_eq!(installing["okfNode"], "topics/installing-product.md");
        assert_eq!(installing["audience"], serde_json::json!(["admin"]));
    }

    #[test]
    fn chunk_text_is_omitted_when_neither_shortdesc_nor_body_present() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_rag_index(&nodes, dir.path(), generated_at).unwrap();

        let raw = fs::read_to_string(dir.path().join("rag/chunks.jsonl")).unwrap();
        let configuration: serde_json::Value = raw
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .find(|v: &serde_json::Value| v["id"] == "configuration")
            .unwrap();
        assert!(configuration.get("text").is_none());
    }

    #[test]
    fn metadata_json_reports_chunk_count() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let generated_at: DateTime<Utc> = "2026-08-03T00:00:00Z".parse().unwrap();
        write_rag_index(&nodes, dir.path(), generated_at).unwrap();

        let metadata: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("rag/metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["chunkCount"], 2);
        assert_eq!(metadata["generated"]["by"], crate::okf::producer());
    }
}
