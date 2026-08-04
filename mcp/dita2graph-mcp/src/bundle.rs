//! Minimal reader over a DITA2Graph OKF bundle: `graph.json` for edges,
//! plus lazy frontmatter/body reads from `okf/{topics,maps}/{id}.md`.
//!
//! Deliberately not `okf_parser::read_bundle` — per
//! `docs/dev/phase-0-findings.md`, that reconstructs the typed
//! `okf_parser::Concept` model, which can't represent DITA topic types or
//! the DITA relation taxonomy (§4.1). This reads our own frontmatter
//! shape (§4.4) generically instead.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct GraphJson {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

#[derive(Deserialize, Clone)]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Deserialize, Clone)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
}

/// One record from `rag/chunks.jsonl` (§13.1) -- the content-search
/// artifact `dita2graph-core build` writes alongside `okf/`, from the
/// same normalized model, not a second parse of the DITA source.
#[derive(Deserialize, Clone)]
pub struct RagChunk {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(rename = "topicType")]
    pub topic_type: String,
}

pub struct BundleReader {
    /// The directory containing `okf/` and `graph.json` (i.e. what
    /// `dita2graph-core build --output` pointed at).
    root: PathBuf,
    nodes: HashMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
}

impl BundleReader {
    pub fn open(root: &Path) -> Result<Self> {
        let graph_path = root.join("graph.json");
        let raw = fs::read_to_string(&graph_path).with_context(|| {
            format!(
                "reading {} (run `dita2graph-core build` first)",
                graph_path.display()
            )
        })?;
        let graph: GraphJson = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", graph_path.display()))?;
        Ok(BundleReader {
            root: root.to_path_buf(),
            nodes: graph.nodes.into_iter().map(|n| (n.id.clone(), n)).collect(),
            edges: graph.edges,
        })
    }

    pub fn all_nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.nodes.values()
    }

    pub fn edges_from(&self, id: &str, relation: Option<&str>) -> Vec<&GraphEdge> {
        self.edges
            .iter()
            .filter(|e| e.from == id && relation.is_none_or(|r| e.relation == r))
            .collect()
    }

    /// The reverse of [`edges_from`](Self::edges_from): every edge that
    /// points *at* `id`, i.e. what depends on it -- the basis for impact
    /// analysis (§13.1).
    pub fn edges_to(&self, id: &str, relation: Option<&str>) -> Vec<&GraphEdge> {
        self.edges
            .iter()
            .filter(|e| e.to == id && relation.is_none_or(|r| e.relation == r))
            .collect()
    }

    /// `okf/topics/{id}.md` or `okf/maps/{id}.md`, whichever exists —
    /// `graph.json` alone doesn't record which subdirectory a node lives
    /// in (§2.4), so both are tried.
    pub fn concept_path(&self, id: &str) -> Option<PathBuf> {
        for subdir in ["topics", "maps"] {
            let path = self.root.join("okf").join(subdir).join(format!("{id}.md"));
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    /// Splits a concept file into (frontmatter YAML, body markdown).
    pub fn read_concept(&self, id: &str) -> Result<(serde_yaml::Value, String)> {
        let path = self
            .concept_path(id)
            .with_context(|| format!("no concept file found for id `{id}`"))?;
        let content =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let content = content.strip_prefix("---\n").unwrap_or(&content);
        let (yaml, body) = content
            .split_once("\n---\n")
            .with_context(|| format!("{} has no frontmatter delimiter", path.display()))?;
        let frontmatter: serde_yaml::Value = serde_yaml::from_str(yaml)
            .with_context(|| format!("parsing frontmatter in {}", path.display()))?;
        Ok((frontmatter, body.trim_start().to_string()))
    }

    /// Loads `rag/chunks.jsonl` (§13.1). Returns an empty `Vec`, not an
    /// error, when the file is missing -- a bundle built before `rag/`
    /// existed, or built by a `dita2graph-core` that predates it, should
    /// degrade content search to "no results" rather than fail every
    /// tool call that touches it.
    pub fn rag_chunks(&self) -> Result<Vec<RagChunk>> {
        let path = self.root.join("rag").join("chunks.jsonl");
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => return Ok(Vec::new()),
        };
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).with_context(|| format!("parsing {}", path.display()))
            })
            .collect()
    }

    pub fn title(&self, id: &str) -> Result<String> {
        let (frontmatter, _) = self.read_concept(id)?;
        Ok(frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_string())
    }
}
