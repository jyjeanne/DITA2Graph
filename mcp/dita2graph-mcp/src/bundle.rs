//! Minimal reader over a DITA2Graph OKF bundle: `graph.json` for edges,
//! plus lazy frontmatter/body reads from `okf/{topics,maps}/{id}.md`.
//!
//! Deliberately not `okf_parser::read_bundle` — per
//! `docs/dev/phase-0-findings.md`, that reconstructs the typed
//! `okf_parser::Concept` model, which can't represent DITA topic types or
//! the DITA relation taxonomy (§4.1). This reads our own frontmatter
//! shape (§4.4) generically instead.
//!
//! [`BundleCache`] is the process-lifetime wrapper `main.rs` actually
//! holds: on a real, sizeable DITA dataset (the reason this exists —
//! see its own docs), reopening and reparsing `graph.json`/
//! `rag/chunks.jsonl`/every touched concept file on *every single* MCP
//! tool call doesn't scale with corpus size, and a real agent session
//! against a real bundle issues many tool calls, not one.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// Per-id `read_concept` results, populated on first read within
    /// this `BundleReader`'s lifetime. `title()` goes through
    /// `read_concept` too, so it benefits automatically -- and it's the
    /// biggest beneficiary: `search_topics` calls it once per node in
    /// the whole bundle on every invocation, so on a real, sizeable
    /// corpus an uncached title lookup means re-reading and re-parsing
    /// every single concept file on every `search_topics` call, not
    /// just the ones a query actually matches. `RefCell` because the
    /// public read methods only need `&self` (tool functions take
    /// `&BundleReader`, not `&mut`) -- interior mutability for a cache
    /// that never affects observable results, only how many times the
    /// filesystem gets touched to produce them.
    concept_cache: RefCell<HashMap<String, (serde_yaml::Value, String)>>,
    /// `rag/chunks.jsonl` (§13.1), parsed once and cloned on repeat
    /// calls rather than re-read from disk -- it holds full body text
    /// for every chunked topic, so it's typically the single largest
    /// file a real bundle has, and `search_content`/`analyze_impact`
    /// both call `rag_chunks()` on every invocation.
    rag_chunks_cache: RefCell<Option<Vec<RagChunk>>>,
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
            concept_cache: RefCell::new(HashMap::new()),
            rag_chunks_cache: RefCell::new(None),
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

    /// Splits a concept file into (frontmatter YAML, body markdown) --
    /// cached per id after the first read (see `concept_cache` above),
    /// since the same `BundleReader` now serves every tool call for as
    /// long as the bundle on disk stays unchanged (`BundleCache`,
    /// below), not just one.
    pub fn read_concept(&self, id: &str) -> Result<(serde_yaml::Value, String)> {
        if let Some(cached) = self.concept_cache.borrow().get(id) {
            return Ok(cached.clone());
        }
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
        let result = (frontmatter, body.trim_start().to_string());
        self.concept_cache
            .borrow_mut()
            .insert(id.to_string(), result.clone());
        Ok(result)
    }

    /// Loads `rag/chunks.jsonl` (§13.1), parsed once and cloned on every
    /// call after that (see `rag_chunks_cache` above). Returns an empty
    /// `Vec`, not an error, when the file is missing -- a bundle built
    /// before `rag/` existed, or built by a `dita2graph-core` that
    /// predates it, should degrade content search to "no results"
    /// rather than fail every tool call that touches it; that empty
    /// result is cached too, so a bundle without `rag/` doesn't retry
    /// the failed read on every call either.
    pub fn rag_chunks(&self) -> Result<Vec<RagChunk>> {
        if let Some(cached) = self.rag_chunks_cache.borrow().as_ref() {
            return Ok(cached.clone());
        }
        let path = self.root.join("rag").join("chunks.jsonl");
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => {
                *self.rag_chunks_cache.borrow_mut() = Some(Vec::new());
                return Ok(Vec::new());
            }
        };
        let chunks: Vec<RagChunk> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).with_context(|| format!("parsing {}", path.display()))
            })
            .collect::<Result<_>>()?;
        *self.rag_chunks_cache.borrow_mut() = Some(chunks.clone());
        Ok(chunks)
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

/// The process-lifetime handle `main.rs` holds across every JSON-RPC
/// request, instead of calling `BundleReader::open` fresh per
/// `tools/call` the way this server originally did. On a real, sizeable
/// DITA dataset that mattered: a real agent session issues many tool
/// calls against the same bundle (`search_topics` to find an id, then
/// several `find_related_topics`/`trace_dependencies`/`analyze_impact`
/// calls to explore from it), and every one of those was independently
/// re-reading and re-parsing `graph.json` (plus, per the caches above,
/// every concept file and `rag/chunks.jsonl` too) from scratch.
///
/// Reopens the underlying `BundleReader` when `graph.json`'s mtime has
/// changed since the last open (or on first use) -- one cheap `stat()`
/// per call in the common case (many tool calls, unchanged bundle),
/// full reparse only when the bundle was actually rebuilt mid-session.
/// If that reopen attempt itself fails and a previous `BundleReader` is
/// already cached, the stale one keeps serving rather than the call
/// failing outright -- a `dita2graph-core build` in progress can leave
/// `graph.json` transiently missing or mid-write, and one MCP tool call
/// landing in that window shouldn't break an otherwise-working session;
/// the next call (or the one after) retries once the rebuild settles.
/// A *first* open failing (no cached reader to fall back to) still
/// propagates, same as `BundleReader::open` always has.
pub struct BundleCache {
    root: PathBuf,
    loaded: Option<(Option<SystemTime>, BundleReader)>,
}

impl BundleCache {
    pub fn new(root: PathBuf) -> Self {
        BundleCache { root, loaded: None }
    }

    /// The bundle root this cache is bound to -- needed directly (not
    /// through the cached `BundleReader`) by `validate_bundle`, which is
    /// meant to check the bundle's *current* on-disk state, not a cached
    /// snapshot of it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn get(&mut self) -> Result<&BundleReader> {
        let mtime = fs::metadata(self.root.join("graph.json"))
            .and_then(|m| m.modified())
            .ok();
        let stale = match &self.loaded {
            Some((cached_mtime, _)) => mtime.is_none() || mtime != *cached_mtime,
            None => true,
        };
        if stale {
            match BundleReader::open(&self.root) {
                Ok(reader) => self.loaded = Some((mtime, reader)),
                Err(e) if self.loaded.is_some() => {
                    eprintln!(
                        "dita2graph-mcp: reload of {} failed, serving previously loaded bundle: {e:#}",
                        self.root.display()
                    );
                }
                Err(e) => return Err(e),
            }
        }
        Ok(&self
            .loaded
            .as_ref()
            .expect("just loaded or already cached")
            .1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dita2graph_core::{
        Link, NormalizedMap, NormalizedNode, NormalizedTopic, Relation, TopicType, write_bundle,
        write_rag_index,
    };
    use std::{fs, thread, time::Duration};

    fn one_topic_bundle(dir: &Path, topic_id: &str, title: &str) {
        let nodes = vec![
            NormalizedNode::Map(NormalizedMap {
                id: "user-guide".into(),
                title: "User Guide".into(),
                source_file: "user-guide.ditamap".into(),
                links: vec![Link {
                    relation: Relation::Contains,
                    target: topic_id.into(),
                }],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: topic_id.into(),
                topic_type: TopicType::Concept,
                title: title.into(),
                shortdesc: None,
                body: Some("Some body text.".into()),
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: format!("topics/{topic_id}.dita"),
                links: vec![],
            }),
        ];
        write_bundle(&nodes, dir, chrono::Utc::now(), true).unwrap();
        write_rag_index(&nodes, dir, chrono::Utc::now()).unwrap();
    }

    /// Proves `read_concept`/`title` are actually cached, not just
    /// correct on a single call: editing the concept file on disk after
    /// the first read must not change what the *same* `BundleReader`
    /// returns on a second call for the same id.
    #[test]
    fn concept_cache_serves_the_first_read_even_after_the_file_changes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        one_topic_bundle(dir.path(), "topic-a", "Original Title");
        let reader = BundleReader::open(dir.path()).unwrap();
        assert_eq!(reader.title("topic-a").unwrap(), "Original Title");

        let path = reader.concept_path("topic-a").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        fs::write(&path, content.replace("Original Title", "Changed Title")).unwrap();

        assert_eq!(
            reader.title("topic-a").unwrap(),
            "Original Title",
            "second call on the same BundleReader should serve the cached read, not the edit"
        );
    }

    /// Same proof as above, for `rag_chunks()`.
    #[test]
    fn rag_chunks_cache_serves_the_first_read_even_after_the_file_changes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        one_topic_bundle(dir.path(), "topic-a", "Title");
        let reader = BundleReader::open(dir.path()).unwrap();
        let first = reader.rag_chunks().unwrap();
        assert_eq!(first.len(), 1);

        fs::write(dir.path().join("rag/chunks.jsonl"), "").unwrap();

        let second = reader.rag_chunks().unwrap();
        assert_eq!(
            second.len(),
            1,
            "second call on the same BundleReader should serve the cached parse, not the now-empty file"
        );
    }

    /// `BundleCache::get()` must still pick up a real rebuild -- caching
    /// for repeat calls against an *unchanged* bundle must not turn into
    /// permanently stale data once the bundle actually changes.
    #[test]
    fn bundle_cache_reloads_when_graph_json_is_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        one_topic_bundle(dir.path(), "topic-a", "Title");
        let mut cache = BundleCache::new(dir.path().to_path_buf());
        assert!(cache.get().unwrap().all_nodes().any(|n| n.id == "topic-a"));

        // Filesystem mtime resolution can be coarser than the time this
        // test takes to run twice in a row -- sleep past it so the
        // rebuild is guaranteed to produce a strictly later mtime.
        thread::sleep(Duration::from_millis(1100));
        one_topic_bundle(dir.path(), "topic-b", "Title B");

        assert!(
            cache.get().unwrap().all_nodes().any(|n| n.id == "topic-b"),
            "cache should reload once graph.json's mtime changes"
        );
    }

    /// If a reopen attempt fails (e.g. a rebuild deletes `graph.json`
    /// before rewriting it) but a bundle was already loaded, the cache
    /// should keep serving that last-known-good bundle rather than
    /// failing the call outright.
    #[test]
    fn bundle_cache_falls_back_to_the_last_loaded_bundle_when_reopen_fails() {
        let dir = tempfile::tempdir().unwrap();
        one_topic_bundle(dir.path(), "topic-a", "Title");
        let mut cache = BundleCache::new(dir.path().to_path_buf());
        assert!(cache.get().unwrap().all_nodes().any(|n| n.id == "topic-a"));

        fs::remove_file(dir.path().join("graph.json")).unwrap();

        let reader = cache
            .get()
            .expect("a reopen failure with a bundle already cached should not surface as an error");
        assert!(reader.all_nodes().any(|n| n.id == "topic-a"));
    }

    /// The inverse of the fallback case above: a *first* open failing
    /// (nothing cached yet to fall back to) must still propagate, same
    /// as `BundleReader::open` always has.
    #[test]
    fn bundle_cache_propagates_the_error_when_the_first_open_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = BundleCache::new(dir.path().to_path_buf());
        assert!(cache.get().is_err());
    }
}
