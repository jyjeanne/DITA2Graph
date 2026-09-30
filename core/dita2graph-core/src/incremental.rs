//! `<output_dir>/build-state.json`: a per-id fingerprint recorded at the
//! end of every `build`, read back at the start of the next one so
//! `okf::write_bundle` can tell which nodes are unchanged since the last
//! build and skip rewriting their concept file -- the first piece of
//! Phase 6+'s "Incremental rebuild" backlog item (`Roadmap.md`).
//!
//! Deliberately hashes, not the nodes themselves: an earlier version of
//! this design persisted the previous build's full `Vec<NormalizedNode>`
//! (reusing its derived `PartialEq` for the comparison, no hash function
//! needed) and compared directly, but that means a second copy of every
//! topic's body text -- the same content `okf/`/`rag/` already carry and
//! already get scanned for secrets (§6.4) -- sitting in a third file
//! `run_build`'s existing `scan_and_report`/`scan_rag_and_report` calls
//! never look at. A hash reveals nothing, so `build-state.json` needs no
//! secret scan of its own.
//!
//! A node's own fields aren't the whole story, though: `render_concept`
//! (`okf.rs`) prints every link target's *title* inline (`- [Configuration
//! Overview](../topics/configuration.md)`), so a topic `A` whose own
//! content is untouched can still need re-rendering if some topic `B` it
//! links to had its title changed -- `A`'s file would otherwise keep
//! showing `B`'s stale title indefinitely, since nothing about `A` itself
//! ever changes again to trigger a fresh render. [`NodeState`] therefore
//! records each node's title (and bundle subdirectory, for the same
//! reason -- both feed the link text `render_concept` writes into every
//! *referencing* node's file, not just the node's own) alongside its
//! content hash, and [`is_unchanged`] checks both: the node's own hash,
//! and every link target's recorded title/subdirectory against what
//! `index` (the current build's id -> (subdir, title) map) says now.
//!
//! Only `okf::write_bundle`'s concept-file-skip decision uses this
//! module -- `embeddings.rs`'s own skip decision (`PreviousEmbeddings`)
//! is scoped more precisely, to just the exact text that gets embedded,
//! and reads `rag/chunks.jsonl`/`rag/embeddings.jsonl` directly instead
//! (see that module's docs for why the two use different mechanisms).

use crate::model::NormalizedNode;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// A hex-encoded content hash of `node`'s own fields (via its
/// `Serialize` impl) -- covers everything that affects its *own*
/// rendered concept file (title, body, links, audience/product, etc.),
/// so any change relevant to that file's content changes this hash.
/// Built on `DefaultHasher` (SipHash) rather than a dedicated hash
/// crate: this value is never compared across a Rust toolchain upgrade
/// in a way that would be *unsafe* if it changed -- `DefaultHasher`'s
/// algorithm isn't guaranteed stable across compiler versions, but a
/// spurious mismatch after a toolchain upgrade just means one
/// unnecessary rewrite of every concept file, not stale content ever
/// being served. Over-approximating "changed" is always safe here;
/// under-approximating it never happens, since a hash collision would
/// need two different serializations to hash equal, not just look
/// similar.
fn content_hash(node: &NormalizedNode) -> Result<String> {
    let bytes = serde_json::to_vec(node).context("serializing node for content hash")?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

/// What [`is_unchanged`] needs to know about one node from the *previous*
/// build: its own content hash, plus the title/subdirectory every
/// *referencing* node's rendered link text depends on (see module docs).
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct NodeState {
    #[serde(rename = "hash")]
    content_hash: String,
    title: String,
    #[serde(rename = "dir")]
    bundle_dir: String,
}

/// id -> [`NodeState`], as persisted in `build-state.json`.
pub(crate) type BuildState = HashMap<String, NodeState>;

/// Loads `<output_dir>/build-state.json`, the previous build's
/// fingerprints. Returns an empty map -- not an error -- when the file
/// is missing (first build) or unparseable (e.g. written by an
/// incompatible future version): either way, every node then compares as
/// "changed" against an absent previous entry, which just means a full
/// rewrite, the same behavior this build would have had before
/// incremental rebuild existed.
pub(crate) fn load_state(output_dir: &Path) -> BuildState {
    let path = output_dir.join("build-state.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return BuildState::new(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Whether `node` can skip re-rendering: its own content hash matches
/// what `previous` (the last build's [`load_state`] result) recorded for
/// its id, *and* every node it links to still has the same title and
/// bundle subdirectory `previous` recorded for it -- `index` is the
/// current build's id -> (subdir, title) map (`okf::write_bundle`'s own),
/// used to read each link target's *current* values for that comparison.
/// `false` for an id `previous` has never seen at all, same as any other
/// change.
fn is_unchanged(
    previous: &BuildState,
    node: &NormalizedNode,
    index: &BTreeMap<&str, (&str, &str)>,
) -> Result<bool> {
    let Some(state) = previous.get(node.id()) else {
        return Ok(false);
    };
    if state.content_hash != content_hash(node)? {
        return Ok(false);
    }
    for link in node.links() {
        let current = index.get(link.target.as_str()).copied();
        match previous.get(link.target.as_str()) {
            Some(target_state) => {
                let (current_dir, current_title) =
                    current.unwrap_or(("topics", link.target.as_str()));
                if target_state.title != current_title || target_state.bundle_dir != current_dir {
                    return Ok(false);
                }
            }
            None => {
                // A target `previous` never recorded: node's own hash
                // already covers its links list, so this only fires when
                // the target is one `node` already linked to before too
                // (else node's own hash would have changed and returned
                // above already). If it's still not a real node now
                // either, it was dangling then and is dangling now --
                // `render_concept`'s own fallback (falling back to the
                // raw id as the title) renders identically either way, so
                // there is nothing here that could have changed. Only
                // when the target *has* since become a real node --
                // `current` now `Some` -- does its rendered title/dir
                // become an actual unknown, so treat that as "changed"
                // rather than risk showing a wrong title.
                if current.is_some() {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// Whether `node` can skip re-rendering *and* the file `path` it would
/// have rendered to actually still exists -- combines [`is_unchanged`]
/// with that existence check, since a hash match against a concept file
/// that's missing for some other reason (hand-deleted, a previous
/// interrupted build) must never be treated as "nothing to do".
pub(crate) fn can_skip(
    previous: &BuildState,
    node: &NormalizedNode,
    index: &BTreeMap<&str, (&str, &str)>,
    path: &Path,
) -> Result<bool> {
    Ok(path.exists() && is_unchanged(previous, node, index)?)
}

/// Writes `<output_dir>/build-state.json`: every current node's
/// fingerprint, replacing whatever was there before -- always a full
/// rewrite of this small index (same "always fresh, cheap to regenerate"
/// discipline `graph.json`/`graph.db` already follow), so an id removed
/// since the last build doesn't linger and compare "unchanged" against a
/// topic that no longer exists.
pub(crate) fn write_state(nodes: &[NormalizedNode], output_dir: &Path) -> Result<()> {
    let mut state = BuildState::with_capacity(nodes.len());
    for node in nodes {
        state.insert(
            node.id().to_string(),
            NodeState {
                content_hash: content_hash(node)?,
                title: node.title().to_string(),
                bundle_dir: node.bundle_dir().to_string(),
            },
        );
    }
    let json = serde_json::to_string_pretty(&state).context("serializing build-state.json")?;
    fs::write(output_dir.join("build-state.json"), json + "\n").context("writing build-state.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Link, NormalizedTopic, Relation, TopicType};

    fn topic(id: &str, title: &str) -> NormalizedNode {
        NormalizedNode::Topic(NormalizedTopic {
            id: id.into(),
            topic_type: TopicType::Concept,
            title: title.into(),
            shortdesc: None,
            body: None,
            audience: vec![],
            product: vec![],
            keys: vec![],
            uicontrols: vec![],
            cmd_uicontrols: vec![],
            source_file: format!("topics/{id}.dita"),
            links: vec![],
        })
    }

    fn topic_linking_to(id: &str, title: &str, target: &str) -> NormalizedNode {
        let mut node = topic(id, title);
        if let NormalizedNode::Topic(t) = &mut node {
            t.links.push(Link {
                relation: Relation::Requires,
                target: target.into(),
            });
        }
        node
    }

    fn index_of(nodes: &[NormalizedNode]) -> BTreeMap<&str, (&str, &str)> {
        nodes
            .iter()
            .map(|n| (n.id(), (n.bundle_dir(), n.title())))
            .collect()
    }

    #[test]
    fn identical_nodes_hash_identically() {
        let a = topic("t", "Title");
        let b = topic("t", "Title");
        assert_eq!(content_hash(&a).unwrap(), content_hash(&b).unwrap());
    }

    #[test]
    fn a_changed_field_changes_the_hash() {
        let a = topic("t", "Title");
        let b = topic("t", "Different Title");
        assert_ne!(content_hash(&a).unwrap(), content_hash(&b).unwrap());
    }

    #[test]
    fn load_state_returns_empty_map_when_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_state(dir.path()).is_empty());
    }

    #[test]
    fn load_state_returns_empty_map_on_unparseable_content() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("build-state.json"), "not json").unwrap();
        assert!(load_state(dir.path()).is_empty());
    }

    #[test]
    fn write_then_load_round_trips_and_reports_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = vec![topic("a", "A"), topic("b", "B")];
        write_state(&nodes, dir.path()).unwrap();

        let loaded = load_state(dir.path());
        assert_eq!(loaded.len(), 2);
        let index = index_of(&nodes);
        assert!(is_unchanged(&loaded, &nodes[0], &index).unwrap());
        assert!(is_unchanged(&loaded, &nodes[1], &index).unwrap());
    }

    #[test]
    fn is_unchanged_is_false_for_an_id_never_seen_before() {
        let previous = BuildState::new();
        let nodes = vec![topic("new", "New")];
        assert!(!is_unchanged(&previous, &nodes[0], &index_of(&nodes)).unwrap());
    }

    #[test]
    fn is_unchanged_is_false_once_the_nodes_own_content_changes() {
        let dir = tempfile::tempdir().unwrap();
        write_state(&[topic("t", "Original")], dir.path()).unwrap();
        let previous = load_state(dir.path());

        let nodes = vec![topic("t", "Changed")];
        assert!(!is_unchanged(&previous, &nodes[0], &index_of(&nodes)).unwrap());
    }

    /// The correctness case this module's own docs call out:
    /// `render_concept` prints a link target's *title* inline, so `a`
    /// must be considered changed when `b` (which `a` links to) gets
    /// renamed, even though `a`'s own fields never touched.
    #[test]
    fn is_unchanged_is_false_when_a_link_targets_title_changes() {
        let dir = tempfile::tempdir().unwrap();
        let old_nodes = vec![topic_linking_to("a", "A", "b"), topic("b", "Old Title")];
        write_state(&old_nodes, dir.path()).unwrap();
        let previous = load_state(dir.path());

        // a is byte-for-byte identical to before; only b's title changed.
        let new_nodes = vec![topic_linking_to("a", "A", "b"), topic("b", "New Title")];
        let index = index_of(&new_nodes);
        assert!(
            !is_unchanged(&previous, &new_nodes[0], &index).unwrap(),
            "a must be re-rendered so it stops showing b's stale title"
        );
    }

    #[test]
    fn is_unchanged_stays_true_when_an_unrelated_node_changes() {
        let dir = tempfile::tempdir().unwrap();
        let old_nodes = vec![
            topic_linking_to("a", "A", "b"),
            topic("b", "B"),
            topic("c", "C"),
        ];
        write_state(&old_nodes, dir.path()).unwrap();
        let previous = load_state(dir.path());

        // c changes; a doesn't link to c, so a should still be skippable.
        let new_nodes = vec![
            topic_linking_to("a", "A", "b"),
            topic("b", "B"),
            topic("c", "Changed C"),
        ];
        let index = index_of(&new_nodes);
        assert!(is_unchanged(&previous, &new_nodes[0], &index).unwrap());
    }

    /// A link to an id that never exists as a real node (an unresolved
    /// xref, a filtered-out map entry) must not permanently defeat the
    /// skip check: `render_concept`'s own fallback renders the same
    /// "unknown target" text every time this stays dangling, so once
    /// `previous` has ever seen it fail to resolve, seeing that again is
    /// not a change.
    #[test]
    fn is_unchanged_stays_true_across_a_link_that_is_dangling_both_times() {
        let dir = tempfile::tempdir().unwrap();
        let old_nodes = vec![topic_linking_to("a", "A", "nonexistent")];
        write_state(&old_nodes, dir.path()).unwrap();
        let previous = load_state(dir.path());

        // a is byte-for-byte identical; "nonexistent" is still not a real
        // node in this build either.
        let new_nodes = vec![topic_linking_to("a", "A", "nonexistent")];
        let index = index_of(&new_nodes);
        assert!(
            is_unchanged(&previous, &new_nodes[0], &index).unwrap(),
            "a repeatedly-dangling link must not permanently disable the skip"
        );
    }

    /// The complementary case: a link target that was dangling last build
    /// but has since become a real node must still be treated as changed
    /// -- its title/dir are a real, previously-unknown value now, not the
    /// same fallback text as before.
    #[test]
    fn is_unchanged_is_false_once_a_previously_dangling_link_resolves_to_a_real_node() {
        let dir = tempfile::tempdir().unwrap();
        let old_nodes = vec![topic_linking_to("a", "A", "b")];
        write_state(&old_nodes, dir.path()).unwrap();
        let previous = load_state(dir.path());

        // "b" is now a real node with its own title.
        let new_nodes = vec![topic_linking_to("a", "A", "b"), topic("b", "B")];
        let index = index_of(&new_nodes);
        assert!(
            !is_unchanged(&previous, &new_nodes[0], &index).unwrap(),
            "a must be re-rendered once its dangling link resolves to a real title"
        );
    }

    #[test]
    fn can_skip_is_false_when_the_concept_file_is_missing_despite_a_hash_match() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = vec![topic("t", "T")];
        write_state(&nodes, dir.path()).unwrap();
        let previous = load_state(dir.path());

        let missing_path = dir.path().join("does-not-exist.md");
        assert!(!can_skip(&previous, &nodes[0], &index_of(&nodes), &missing_path).unwrap());
    }

    #[test]
    fn rewriting_the_state_drops_an_id_that_no_longer_exists() {
        let dir = tempfile::tempdir().unwrap();
        write_state(&[topic("a", "A"), topic("b", "B")], dir.path()).unwrap();
        write_state(&[topic("a", "A")], dir.path()).unwrap();

        let loaded = load_state(dir.path());
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("a"));
    }
}
