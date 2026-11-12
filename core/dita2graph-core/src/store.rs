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
//! `from`/`to`/`relation`/`fragments` per edge) and nothing more -- no title, no
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
    write_sqlite_view(&crate::graph_view::graph_view(nodes), output_dir)
}

/// [`write_sqlite_store`] from a node/edge view the caller already built
/// (the one `graph.json` is written from).
pub fn write_sqlite_view(
    graph: &crate::graph_view::GraphView,
    output_dir: &Path,
) -> Result<StoreSummary> {
    let db_path = output_dir.join("graph.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path)
            .with_context(|| format!("removing stale {}", db_path.display()))?;
    }
    let mut conn =
        Connection::open(&db_path).with_context(|| format!("creating {}", db_path.display()))?;
    conn.execute_batch(
        "CREATE TABLE nodes (id TEXT PRIMARY KEY, type TEXT NOT NULL, title TEXT, source_file TEXT, processing_role TEXT);
         CREATE TABLE edges (from_id TEXT NOT NULL, to_id TEXT NOT NULL, relation TEXT NOT NULL, fragments TEXT);
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
        // A normalized-model input with two nodes sharing an id is as
        // valid here as for `graph.json`/`okf/`: `okf.rs::write_bundle`
        // has the second node's `.md` file overwrite the first's, and
        // `graph_view` keeps that last node too, so the view holds each
        // id once. `OR REPLACE` stays as the same rule for a view built
        // any other way: a bare `INSERT` would hit `id`'s `PRIMARY KEY`
        // and fail the build only when `--store sqlite` is given.
        let mut insert_node = tx
            .prepare(
                "INSERT OR REPLACE INTO nodes (id, type, title, source_file, processing_role) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("preparing node insert")?;
        let mut insert_edge = tx
            .prepare(
                "INSERT INTO edges (from_id, to_id, relation, fragments) VALUES (?1, ?2, ?3, ?4)",
            )
            .context("preparing edge insert")?;
        // The same nodes and edges as graph.json, keys/URLs/files included.
        for node in &graph.nodes {
            insert_node
                .execute(params![
                    node.id,
                    node.type_,
                    node.title,
                    node.source_file,
                    node.processing_role
                ])
                .with_context(|| format!("inserting node `{}`", node.id))?;
            summary.nodes_written += 1;
        }
        for edge in &graph.edges {
            let fragments = if edge.fragments.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&edge.fragments).context("serializing edge fragments")?)
            };
            insert_edge
                .execute(params![edge.from, edge.to, edge.relation, fragments])
                .with_context(|| format!("inserting edge from `{}`", edge.from))?;
            summary.edges_written += 1;
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

/// `(id, type)`, as dumped by [`read_sqlite_store`].
pub type StoreNode = crate::graph_view::ViewNode;
/// `(from, to, relation, fragments)`, as dumped by [`read_sqlite_store`].
pub type StoreEdge = (String, String, String, Vec<String>);

/// Every node (`id`, `type`) and edge (`from`, `to`, `relation`, `fragments`) in
/// `db_path` -- the same full nodes+edges view `graph.json` itself
/// provides, for a caller that wants to load a bundle straight from
/// `graph.db` instead of parsing `graph.json`, skipping the JSON parse
/// entirely (`mcp/dita2graph-mcp/src/bundle.rs::BundleReader::open`,
/// the "fast MCP queries" motivation §7 documents for this file). Row
/// order is whatever SQLite returns with no `ORDER BY` -- not guaranteed
/// to match `graph.json`'s insertion order, and callers that care about
/// order (none in this codebase do -- `BundleReader`'s own node storage
/// is a `HashMap`, unordered regardless of source) need to sort it
/// themselves.
pub fn read_sqlite_store(db_path: &Path) -> Result<(Vec<StoreNode>, Vec<StoreEdge>)> {
    let conn =
        Connection::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;

    // A graph.db written before keys, URLs and files were nodes has no
    // title/source_file columns, and one written before processing roles
    // has no processing_role: read it as it is.
    let has = |columns: &str| {
        conn.prepare(&format!("SELECT {columns} FROM nodes LIMIT 0"))
            .is_ok()
    };
    let query = if has("processing_role") {
        "SELECT id, type, title, source_file, processing_role FROM nodes"
    } else if has("title, source_file") {
        "SELECT id, type, title, source_file, NULL FROM nodes"
    } else {
        "SELECT id, type, NULL, NULL, NULL FROM nodes"
    };
    let mut node_stmt = conn.prepare(query).context("preparing node dump")?;
    let nodes = node_stmt
        .query_map([], |row| {
            Ok(crate::graph_view::ViewNode {
                id: row.get(0)?,
                type_: row.get(1)?,
                title: row.get(2)?,
                source_file: row.get(3)?,
                processing_role: row.get(4)?,
            })
        })
        .context("reading graph.db nodes")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collecting graph.db nodes")?;
    drop(node_stmt);

    // A graph.db written before the fragments column existed fails here,
    // and BundleReader::open then falls back to graph.json.
    let mut edge_stmt = conn
        .prepare("SELECT from_id, to_id, relation, fragments FROM edges")
        .context("preparing edge dump")?;
    let rows = edge_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .context("reading graph.db edges")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collecting graph.db edges")?;
    let mut edges = Vec::with_capacity(rows.len());
    for (from, to, relation, fragments) in rows {
        let fragments = match fragments {
            Some(json) => serde_json::from_str(&json)
                .with_context(|| format!("parsing fragments of edge {from} -> {to}"))?,
            None => Vec::new(),
        };
        edges.push((from, to, relation, fragments));
    }

    Ok((nodes, edges))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Link, NormalizedMap, NormalizedTopic, Relation, TopicType};

    fn sample_nodes() -> Vec<NormalizedNode> {
        vec![
            NormalizedNode::Map(NormalizedMap {
                key_defs: vec![],
                id: "user-guide".into(),
                title: "User Guide".into(),
                source_file: "user-guide.ditamap".into(),
                links: vec![Link {
                    relation: Relation::Contains,
                    target: "installing-product".into(),
                    fragments: Vec::new(),
                }],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                keyrefs: vec![],
                external_links: vec![],
                code_refs: vec![],
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
                resolved_body: None,
                processing_role: None,
                dita_class: None,
                authored_file: None,
                source_file: "topics/installing-product.dita".into(),
                links: vec![
                    Link {
                        relation: Relation::Requires,
                        target: "configuration".into(),
                        fragments: Vec::new(),
                    },
                    Link {
                        relation: Relation::References,
                        target: "configuration".into(),
                        fragments: Vec::new(),
                    },
                ],
            }),
            NormalizedNode::Topic(NormalizedTopic {
                keyrefs: vec![],
                external_links: vec![],
                code_refs: vec![],
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
                resolved_body: None,
                processing_role: None,
                dita_class: None,
                authored_file: None,
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
            keyrefs: vec![],
            external_links: vec![],
            code_refs: vec![],
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
            resolved_body: None,
            processing_role: None,
            dita_class: None,
            authored_file: None,
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
                key_defs: vec![],
                id: "dup".into(),
                title: "First".into(),
                source_file: "a.ditamap".into(),
                links: vec![],
            }),
            NormalizedNode::Map(NormalizedMap {
                key_defs: vec![],
                id: "dup".into(),
                title: "Second".into(),
                source_file: "b.ditamap".into(),
                links: vec![],
            }),
        ];
        // The view graph.json and graph.db share keeps the first of two
        // nodes with one id, so only one row is ever written.
        let summary = write_sqlite_store(&nodes, dir.path()).unwrap();
        assert_eq!(summary.nodes_written, 1);

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

    #[test]
    fn generated_from_fragments_round_trip_through_graph_db() {
        let dir = tempfile::tempdir().unwrap();
        let mut nodes = sample_nodes();
        if let NormalizedNode::Topic(t) = &mut nodes[1] {
            t.links.push(Link {
                relation: Relation::GeneratedFrom,
                target: "configuration".into(),
                fragments: vec!["warning-note".into(), "li@37".into()],
            });
        }
        write_sqlite_store(&nodes, dir.path()).unwrap();

        let (_, edges) = read_sqlite_store(&dir.path().join("graph.db")).unwrap();
        let generated_from = edges
            .iter()
            .find(|(_, _, relation, _)| relation == "generated-from")
            .expect("the generated-from edge");
        assert_eq!(
            generated_from.3,
            vec!["warning-note".to_string(), "li@37".to_string()]
        );
    }

    #[test]
    fn read_sqlite_store_dumps_every_node_and_edge() {
        let dir = tempfile::tempdir().unwrap();
        write_sqlite_store(&sample_nodes(), dir.path()).unwrap();

        let (nodes, edges) = read_sqlite_store(&dir.path().join("graph.db")).unwrap();
        let mut nodes: Vec<(String, String)> = nodes.into_iter().map(|n| (n.id, n.type_)).collect();
        nodes.sort();
        assert!(
            edges
                .iter()
                .all(|(_, _, _, fragments)| fragments.is_empty())
        );
        let mut edges: Vec<_> = edges
            .into_iter()
            .map(|(from, to, relation, _)| (from, to, relation))
            .collect();
        edges.sort();
        assert_eq!(
            nodes,
            vec![
                ("configuration".to_string(), "Concept".to_string()),
                ("installing-product".to_string(), "Task".to_string()),
                ("user-guide".to_string(), "DITA Map".to_string()),
            ]
        );
        assert_eq!(
            edges,
            vec![
                (
                    "installing-product".to_string(),
                    "configuration".to_string(),
                    "references".to_string()
                ),
                (
                    "installing-product".to_string(),
                    "configuration".to_string(),
                    "requires".to_string()
                ),
                (
                    "user-guide".to_string(),
                    "installing-product".to_string(),
                    "contains".to_string()
                ),
            ]
        );
    }
}
