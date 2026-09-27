//! Writes `<output_dir>/graph.db`: a SQLite-backed mirror of the same
//! nodes/edges `graph.json` (`okf.rs::write_graph_json`) already writes
//! from the same in-memory normalized model -- an indexed copy for fast
//! lookups on a real, sizeable corpus, not a second source of truth
//! (§7's "Storage" row: "derived index only -- the bundle itself is
//! markdown").
//!
//! First step of Phase 6+'s "Incremental rebuild ... and SQLite/RocksDB
//! storage for the query index" backlog item (`Roadmap.md`) -- storage
//! only. Incremental rebuild (diffing against an existing store on
//! rebuild, keyed by source-file hash, so an unchanged topic isn't
//! rewritten) is separate, later work: it needs a persistent store to
//! diff against, which this provides, but this module always rebuilds
//! `graph.db` from scratch on every `build` -- there is no diffing here
//! yet.
//!
//! Exactly mirrors `graph.json`'s own fields (`id`/`type` per node,
//! `from`/`to`/`relation` per edge) and nothing more -- no title, no
//! generated-at metadata, neither of which `graph.json` carries either
//! (`BundleReader`/`run_query` both already get a title, when they need
//! one, from the `okf/` concept file itself, not from the graph index).

use crate::model::NormalizedNode;
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::Path;

/// A written store's summary, for CLI reporting (`dita2graph-core
/// build`), mirroring [`crate::rag::RagSummary`]/[`crate::embeddings::EmbeddingSummary`].
#[derive(Debug, Default)]
pub struct StoreSummary {
    pub nodes_written: usize,
    pub edges_written: usize,
}

/// Writes `<output_dir>/graph.db` from `nodes`, replacing any existing
/// file at that path. Always a full rewrite, not an incremental update
/// (see module docs) -- a stale row for an id removed since the last
/// build must not survive, and starting from a clean file is the
/// simplest way to guarantee that until real incremental rebuild exists.
pub fn write_sqlite_store(nodes: &[NormalizedNode], output_dir: &Path) -> Result<StoreSummary> {
    let db_path = output_dir.join("graph.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path)
            .with_context(|| format!("removing stale {}", db_path.display()))?;
    }
    let mut conn =
        Connection::open(&db_path).with_context(|| format!("creating {}", db_path.display()))?;
    conn.execute_batch(
        "CREATE TABLE nodes (id TEXT PRIMARY KEY, type TEXT NOT NULL);
         CREATE TABLE edges (from_id TEXT NOT NULL, to_id TEXT NOT NULL, relation TEXT NOT NULL);
         CREATE INDEX idx_edges_from ON edges(from_id, relation);
         CREATE INDEX idx_edges_to ON edges(to_id, relation);",
    )
    .context("creating graph.db schema")?;

    let mut summary = StoreSummary::default();
    // One transaction for the whole write, not one commit per row --
    // on a real, sizeable corpus (thousands of topics/edges) SQLite's
    // per-statement autocommit fsync would dominate build time.
    let tx = conn
        .transaction()
        .context("starting graph.db write transaction")?;
    {
        // `OR REPLACE`, not a plain `INSERT`: a normalized-model input
        // with two nodes sharing an id is exactly as valid here as it is
        // for `graph.json`/`okf/` (`okf.rs::write_bundle` just has the
        // second node's `.md` file overwrite the first's at the same
        // path -- last node wins, no error). A bare `INSERT` would hit
        // `id`'s `PRIMARY KEY` and turn that into a hard `build` failure
        // that only happens when `--store sqlite` is given, diverging
        // from the default path for the same input.
        let mut insert_node = tx
            .prepare("INSERT OR REPLACE INTO nodes (id, type) VALUES (?1, ?2)")
            .context("preparing node insert")?;
        let mut insert_edge = tx
            .prepare("INSERT INTO edges (from_id, to_id, relation) VALUES (?1, ?2, ?3)")
            .context("preparing edge insert")?;
        for node in nodes {
            insert_node
                .execute(params![node.id(), node.okf_type()])
                .with_context(|| format!("inserting node `{}`", node.id()))?;
            summary.nodes_written += 1;
            for link in node.links() {
                insert_edge
                    .execute(params![node.id(), link.target, link.relation.as_str()])
                    .with_context(|| format!("inserting edge from `{}`", node.id()))?;
                summary.edges_written += 1;
            }
        }
    }
    tx.commit().context("committing graph.db")?;
    Ok(summary)
}

/// Forward edges from `topic` in `db_path` (an existing `graph.db`),
/// optionally filtered to `relation` -- the SQLite-backed equivalent of
/// `main.rs::run_query`'s existing graph.json-reading path, same
/// `(relation, target)` shape.
pub fn query_sqlite_store(
    db_path: &Path,
    topic: &str,
    relation: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let conn =
        Connection::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;
    let mut stmt = match relation {
        Some(_) => conn
            .prepare("SELECT relation, to_id FROM edges WHERE from_id = ?1 AND relation = ?2")
            .context("preparing scoped edge query")?,
        None => conn
            .prepare("SELECT relation, to_id FROM edges WHERE from_id = ?1")
            .context("preparing edge query")?,
    };
    let rows = if let Some(relation) = relation {
        stmt.query_map(params![topic, relation], row_to_pair)
    } else {
        stmt.query_map(params![topic], row_to_pair)
    }
    .context("querying graph.db")?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("reading graph.db query results")
}

fn row_to_pair(row: &rusqlite::Row) -> rusqlite::Result<(String, String)> {
    Ok((row.get(0)?, row.get(1)?))
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
                shortdesc: None,
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/installing-product.dita".into(),
                links: vec![
                    Link {
                        relation: Relation::Requires,
                        target: "configuration".into(),
                    },
                    Link {
                        relation: Relation::References,
                        target: "configuration".into(),
                    },
                ],
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
    fn writes_every_node_and_edge() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        let summary = write_sqlite_store(&nodes, dir.path()).unwrap();
        assert_eq!(summary.nodes_written, 3);
        assert_eq!(summary.edges_written, 3); // 1 contains + 2 from installing-product
        assert!(dir.path().join("graph.db").exists());
    }

    #[test]
    fn query_unscoped_returns_every_outgoing_edge() {
        let dir = tempfile::tempdir().unwrap();
        write_sqlite_store(&sample_nodes(), dir.path()).unwrap();
        let mut edges =
            query_sqlite_store(&dir.path().join("graph.db"), "installing-product", None).unwrap();
        edges.sort();
        assert_eq!(
            edges,
            vec![
                ("references".to_string(), "configuration".to_string()),
                ("requires".to_string(), "configuration".to_string()),
            ]
        );
    }

    #[test]
    fn query_scoped_to_a_relation_filters_out_the_others() {
        let dir = tempfile::tempdir().unwrap();
        write_sqlite_store(&sample_nodes(), dir.path()).unwrap();
        let edges = query_sqlite_store(
            &dir.path().join("graph.db"),
            "installing-product",
            Some("requires"),
        )
        .unwrap();
        assert_eq!(
            edges,
            vec![("requires".to_string(), "configuration".to_string())]
        );
    }

    #[test]
    fn query_for_a_topic_with_no_outgoing_edges_returns_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write_sqlite_store(&sample_nodes(), dir.path()).unwrap();
        let edges =
            query_sqlite_store(&dir.path().join("graph.db"), "configuration", None).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn rebuilding_replaces_stale_rows_rather_than_appending() {
        let dir = tempfile::tempdir().unwrap();
        write_sqlite_store(&sample_nodes(), dir.path()).unwrap();

        // Rebuild from a model where installing-product no longer exists
        // at all -- a real "topic deleted since the last build" case.
        let smaller = vec![NormalizedNode::Topic(NormalizedTopic {
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
        })];
        let summary = write_sqlite_store(&smaller, dir.path()).unwrap();
        assert_eq!(summary.nodes_written, 1);
        assert_eq!(summary.edges_written, 0);

        let edges =
            query_sqlite_store(&dir.path().join("graph.db"), "installing-product", None).unwrap();
        assert!(
            edges.is_empty(),
            "a rebuilt store should not still answer queries for a removed topic"
        );
    }

    /// A normalized-model input with two nodes sharing an id is unusual
    /// but not something `graph.json`/`okf/` reject (`okf.rs::write_bundle`
    /// just has the second node's `.md` file overwrite the first's, no
    /// error) -- `--store sqlite` must not turn that same input into a
    /// hard `build` failure via `nodes.id`'s `PRIMARY KEY`.
    #[test]
    fn duplicate_ids_replace_rather_than_error() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = vec![
            NormalizedNode::Map(NormalizedMap {
                id: "dup".into(),
                title: "First".into(),
                source_file: "a.ditamap".into(),
                links: vec![],
            }),
            NormalizedNode::Map(NormalizedMap {
                id: "dup".into(),
                title: "Second".into(),
                source_file: "b.ditamap".into(),
                links: vec![],
            }),
        ];
        let summary = write_sqlite_store(&nodes, dir.path()).unwrap();
        assert_eq!(summary.nodes_written, 2);

        let conn = Connection::open(dir.path().join("graph.db")).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes WHERE id = 'dup'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, 1,
            "the second node should replace the first, not duplicate it"
        );
    }
}
