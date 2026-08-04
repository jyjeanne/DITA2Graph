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
use std::rc::Rc;
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
    /// `rag/chunks.jsonl` (§13.1), parsed once and `Rc`-shared on repeat
    /// calls rather than re-read from disk -- it holds full body text
    /// for every chunked topic, so it's typically the single largest
    /// file a real bundle has, and `search_content`/`analyze_impact`
    /// both call `rag_chunks()` on every invocation. `Rc`, not a plain
    /// `Vec` clone on every call: a real corpus's worth of chunk text is
    /// exactly the data this cache exists to stop re-copying, so a cache
    /// hit needs to be a cheap refcount bump, not a fresh deep clone of
    /// every chunk's owned strings each time.
    rag_chunks_cache: RefCell<Option<Rc<Vec<RagChunk>>>>,
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

    /// When `attempted` doesn't exist but is exactly the un-disambiguated
    /// tail of one real id, suggests that real id -- the shape
    /// `DitaModelExtractor`'s duplicate-topic-id disambiguation
    /// (`DITA2GRAPH070W`) produces is `{original-id}--{source-path}`, and
    /// a real live Claude Code session (not a hypothetical) guessed the
    /// bare `{source-path}` tail after `search_topics` showed it the
    /// full `[ID--topics-troubleshooting-overview.dita]` id -- a
    /// reasonable-looking but wrong guess that a plain "no concept file
    /// found" error gives no way to self-correct from except trial and
    /// error. Only fires on an *unambiguous* single match (mirrors
    /// `relations.rs`'s "an ambiguous match is dropped, not guessed at"
    /// discipline) -- two disambiguated ids can't share a suffix in
    /// practice (the source path is unique per topic), so this is
    /// effectively always unambiguous when it fires at all, but the
    /// check costs nothing and keeps the guarantee explicit.
    fn suggest_id(&self, attempted: &str) -> Option<&str> {
        let suffix = format!("--{attempted}");
        let mut matches = self
            .nodes
            .keys()
            .filter(|id| id.ends_with(&suffix))
            .map(String::as_str);
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
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
            .with_context(|| match self.suggest_id(id) {
                Some(suggestion) => {
                    format!("no concept file found for id `{id}` -- did you mean `{suggestion}`?")
                }
                None => format!("no concept file found for id `{id}`"),
            })?;
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

    /// Loads `rag/chunks.jsonl` (§13.1), parsed once and `Rc`-shared on
    /// every call after that (see `rag_chunks_cache` above) -- a cache
    /// hit is a refcount bump, not a deep clone of every chunk's owned
    /// strings. Returns an empty `Vec`, not an error, when the file is
    /// missing -- a bundle built before `rag/` existed, or built by a
    /// `dita2graph-core` that predates it, should degrade content search
    /// to "no results" rather than fail every tool call that touches it;
    /// that empty result is cached too, so a bundle without `rag/`
    /// doesn't retry the failed read on every call either.
    pub fn rag_chunks(&self) -> Result<Rc<Vec<RagChunk>>> {
        if let Some(cached) = self.rag_chunks_cache.borrow().as_ref() {
            return Ok(Rc::clone(cached));
        }
        let path = self.root.join("rag").join("chunks.jsonl");
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => {
                let empty = Rc::new(Vec::new());
                *self.rag_chunks_cache.borrow_mut() = Some(Rc::clone(&empty));
                return Ok(empty);
            }
        };
        let chunks: Vec<RagChunk> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).with_context(|| format!("parsing {}", path.display()))
            })
            .collect::<Result<_>>()?;
        let chunks = Rc::new(chunks);
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

/// The mtimes `BundleCache` fingerprints a bundle by. Both files, not
/// just `graph.json` -- `dita2graph-core build` (`main.rs::run_build`)
/// writes them in two separate steps, `write_bundle` (which writes
/// `graph.json` last, after every concept file) finishing before
/// `write_rag_index` even starts. Fingerprinting `graph.json` alone
/// would mean a `get()` landing in that window reopens the reader (safe
/// for concept files -- they're all written *before* `graph.json` -- but
/// unsafe for `rag/chunks.jsonl`, which hasn't been rewritten yet at
/// that point) and permanently caches the stale-or-missing rag index
/// into `rag_chunks_cache` until graph.json's mtime changes *again* at
/// the *next* rebuild -- meanwhile `search_content`/`analyze_impact`
/// silently serve outdated excerpts against an otherwise fully
/// up-to-date bundle for the entire rest of the session.
type BundleFingerprint = (Option<SystemTime>, Option<SystemTime>);

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
/// Reopens the underlying `BundleReader` when its [`BundleFingerprint`]
/// has changed since the last open (or on first use) -- two cheap
/// `stat()`s per call in the common case (many tool calls, unchanged
/// bundle), full reparse only when the bundle was actually rebuilt
/// mid-session. If that reopen attempt itself fails and a previous
/// `BundleReader` is already cached, the stale one keeps serving rather
/// than the call failing outright -- a `dita2graph-core build` in
/// progress can leave `graph.json` transiently missing or mid-write, and
/// one MCP tool call landing in that window shouldn't break an
/// otherwise-working session; the next call (or the one after) retries
/// once the rebuild settles. A *first* open failing (no cached reader to
/// fall back to) still propagates, same as `BundleReader::open` always
/// has.
///
/// Known, accepted limitation: this fingerprints the two top-level files
/// every tool call already touches, not every individual `okf/*.md`
/// concept file (an O(1) check regardless of corpus size, preserving the
/// whole point of caching on a large bundle -- an O(topics) directory
/// walk on every call would give most of that back). Concept files are
/// generated output, always rewritten as part of the same `write_bundle`
/// call that rewrites `graph.json` (and always *before* it, so a fresh
/// `graph.json` mtime guarantees fresh concept files too) -- hand-editing
/// one directly, outside a real `dita2graph-core build` run, is already
/// outside this tool's supported workflow, and won't be picked up until
/// the fingerprint next changes for an unrelated reason.
pub struct BundleCache {
    root: PathBuf,
    loaded: Option<(BundleFingerprint, BundleReader)>,
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

    fn fingerprint(&self) -> BundleFingerprint {
        let mtime_of = |relative: &str| {
            fs::metadata(self.root.join(relative))
                .and_then(|m| m.modified())
                .ok()
        };
        (mtime_of("graph.json"), mtime_of("rag/chunks.jsonl"))
    }

    pub fn get(&mut self) -> Result<&BundleReader> {
        let fingerprint = self.fingerprint();
        // Plain inequality, deliberately -- `rag/chunks.jsonl` is
        // legitimately, permanently absent for plenty of real bundles
        // (no rag/ built at all, `search_content_reports_no_rag_index_
        // when_bundle_predates_rag`), so a `None` fingerprint component
        // must compare equal to a previous `None`, not be forced stale
        // on every single call -- that would silently defeat caching
        // entirely for any bundle without a rag index. A component that
        // goes from `Some` to `None` (the file disappeared) or changes
        // value is already caught by this same inequality, no special
        // case needed: `BundleReader::open` re-reads `graph.json` right
        // below regardless, and fails there if it's genuinely gone,
        // landing in the fallback-to-cached-reader arm just like any
        // other reopen failure.
        let stale = match &self.loaded {
            Some((cached_fingerprint, _)) => fingerprint != *cached_fingerprint,
            None => true,
        };
        if stale {
            match BundleReader::open(&self.root) {
                Ok(reader) => self.loaded = Some((fingerprint, reader)),
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

    fn one_topic_nodes(topic_id: &str, title: &str) -> Vec<NormalizedNode> {
        vec![
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
        ]
    }

    fn one_topic_bundle(dir: &Path, topic_id: &str, title: &str) {
        let nodes = one_topic_nodes(topic_id, title);
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

    /// Fingerprinting `graph.json` alone would miss this: `run_build`
    /// (`core/dita2graph-core/src/main.rs`) writes `graph.json` (via
    /// `write_bundle`) and rewrites `rag/chunks.jsonl` (via
    /// `write_rag_index`) as two *separate* steps, the first finishing
    /// before the second starts. A reader whose lazy `rag_chunks_cache`
    /// gets populated in that in-between window -- a real possibility on
    /// a real dataset where an agent session is issuing tool calls while
    /// a rebuild is in flight -- must not go on serving that snapshot
    /// forever just because `graph.json` itself doesn't change again
    /// until the *next* rebuild.
    #[test]
    fn bundle_cache_reloads_when_only_rag_chunks_jsonl_changes_after_graph_json_settled() {
        let dir = tempfile::tempdir().unwrap();
        one_topic_bundle(dir.path(), "topic-a", "Title A");
        let mut cache = BundleCache::new(dir.path().to_path_buf());
        // Poison this reader's lazy rag_chunks_cache with topic-a's chunk.
        let chunks = cache.get().unwrap().rag_chunks().unwrap();
        assert!(chunks.iter().any(|c| c.id == "topic-a"));

        thread::sleep(Duration::from_millis(1100));
        // Simulate write_bundle's half of a rebuild only -- graph.json
        // (and every concept file) now names topic-b, but rag/chunks.jsonl
        // hasn't been touched yet, exactly the window between
        // write_bundle and write_rag_index in run_build.
        write_bundle(
            &one_topic_nodes("topic-b", "Title B"),
            dir.path(),
            chrono::Utc::now(),
            true,
        )
        .unwrap();

        thread::sleep(Duration::from_millis(1100));
        // write_rag_index finally catches up -- graph.json itself is
        // untouched this time, only rag/chunks.jsonl changes.
        write_rag_index(
            &one_topic_nodes("topic-b", "Title B"),
            dir.path(),
            chrono::Utc::now(),
        )
        .unwrap();

        let chunks = cache.get().unwrap().rag_chunks().unwrap();
        assert!(
            chunks.iter().any(|c| c.id == "topic-b"),
            "cache should reload once rag/chunks.jsonl changes even when graph.json didn't change again: {:?}",
            chunks.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
    }

    /// A bundle with no `rag/` at all (predates it, or simply never
    /// built one) must not be treated as permanently stale -- both
    /// fingerprint components are `None` and stay `None`, which must
    /// compare equal to itself, not force a reopen on every single call.
    #[test]
    fn bundle_cache_still_caches_a_bundle_with_no_rag_index() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = one_topic_nodes("topic-a", "Title A");
        write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
        // Deliberately no write_rag_index call.
        let mut cache = BundleCache::new(dir.path().to_path_buf());

        let reader = cache.get().unwrap();
        assert!(reader.rag_chunks().unwrap().is_empty());
        // Poison this reader's concept_cache with the original title
        // *before* editing the file, or there'd be nothing cached yet to
        // prove staleness against.
        assert_eq!(reader.title("topic-a").unwrap(), "Title A");
        let path = reader.concept_path("topic-a").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        fs::write(&path, content.replace("Title A", "Changed Title")).unwrap();

        // If the fingerprint were wrongly treating a missing rag index as
        // "always changed", this next get() would reopen a fresh reader
        // and the edit above would (wrongly, for this specific case)
        // become visible immediately; the cache should still be serving
        // the same, already-cached reader here.

        assert_eq!(
            cache.get().unwrap().title("topic-a").unwrap(),
            "Title A",
            "a bundle with no rag/ should still be cached across calls, not reopened every time"
        );
    }

    /// Caught live, not hypothesized: a real Claude Code session saw
    /// `search_topics` display `[ID--topics-troubleshooting-overview.dita]`
    /// and guessed the bare `topics-troubleshooting-overview.dita` tail
    /// -- a reasonable-looking wrong guess at what
    /// `DitaModelExtractor`'s duplicate-id disambiguation (`DITA2GRAPH070W`)
    /// produces. The error for that guess should point straight at the
    /// real id instead of leaving the caller to trial-and-error it.
    #[test]
    fn read_concept_suggests_the_real_id_for_a_disambiguation_prefix_guess() {
        let dir = tempfile::tempdir().unwrap();
        one_topic_bundle(
            dir.path(),
            "ID--topics-troubleshooting-overview.dita",
            "Troubleshooting",
        );
        let reader = BundleReader::open(dir.path()).unwrap();

        let err = reader
            .read_concept("topics-troubleshooting-overview.dita")
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("did you mean `ID--topics-troubleshooting-overview.dita`"),
            "{err}"
        );
    }

    /// No suggestion at all for a genuinely wrong id that isn't the tail
    /// of any real one -- a made-up guess shouldn't get a confident-
    /// looking "did you mean" pointing nowhere useful.
    #[test]
    fn read_concept_suggests_nothing_for_an_id_with_no_plausible_match() {
        let dir = tempfile::tempdir().unwrap();
        one_topic_bundle(dir.path(), "topic-a", "Title A");
        let reader = BundleReader::open(dir.path()).unwrap();

        let err = reader
            .read_concept("completely-unrelated-guess")
            .unwrap_err();
        assert!(!err.to_string().contains("did you mean"), "{err}");
    }

    /// Two different disambiguated ids sharing the same tail (a real,
    /// if unusual, possibility: two topics with different original ids
    /// that both happen to end in the same source-path suffix) must not
    /// produce a confident single suggestion -- the same "don't guess at
    /// an ambiguous match" discipline `relations.rs`'s `applies-to`
    /// inference already follows.
    #[test]
    fn read_concept_suggests_nothing_when_the_tail_is_ambiguous() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = vec![
            NormalizedNode::Topic(NormalizedTopic {
                id: "first--shared-tail".into(),
                topic_type: TopicType::Concept,
                title: "First".into(),
                shortdesc: None,
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/first.dita".into(),
                links: vec![],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                id: "second--shared-tail".into(),
                topic_type: TopicType::Concept,
                title: "Second".into(),
                shortdesc: None,
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/second.dita".into(),
                links: vec![],
            }),
        ];
        write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
        let reader = BundleReader::open(dir.path()).unwrap();

        let err = reader.read_concept("shared-tail").unwrap_err();
        assert!(!err.to_string().contains("did you mean"), "{err}");
    }
}
