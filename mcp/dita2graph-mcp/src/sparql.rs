//! `query_graph`, phase 5 of the semantic model (`docs/semantic-model.md`):
//! read-only SPARQL 1.1 over the bundle's RDF export, `rdf/graph.ttl`.
//!
//! Opt-in (`dita2graph-mcp --sparql`): the typed tools stay the default,
//! and the tool is adopted only if a benchmark campaign shows agents gain
//! from writing SPARQL. The export is loaded once into an in-memory
//! Oxigraph store (no RocksDB, no HTTP client, so a `SERVICE` clause can
//! reach nothing). Only queries parse -- SELECT, ASK, CONSTRUCT, DESCRIBE;
//! an update is a syntax error -- each one is stopped after a timeout, and
//! results are paginated and their size bounded.

use anyhow::{Context, Result, anyhow, bail};
use oxigraph::io::RdfFormat;
use oxigraph::model::Term;
use oxigraph::sparql::{CancellationToken, QueryEvaluationError, QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Prefixes every query can use without declaring them, besides the
/// export's own ([`dita2graph_core::rdf::PREFIXES`]).
const EXTRA_PREFIXES: &[(&str, &str)] = &[
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
];

/// Rows per page by default, and at most.
pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 500;
/// Results counted, at most (or up to the end of the page asked for):
/// beyond, the total shown is a lower bound.
const COUNT_CAP: usize = 10_000;
/// Characters of a literal shown in a cell: read_topic is for text.
const CELL_CHARS: usize = 200;
/// A query longer than this is refused.
const MAX_QUERY_CHARS: usize = 20_000;
/// How long a query may run.
pub const TIMEOUT: Duration = Duration::from_secs(10);

pub struct SparqlGraph {
    store: Store,
    /// (prefix, namespace): the vocabularies, then this bundle's data IRIs.
    prefixes: Vec<(String, String)>,
}

impl SparqlGraph {
    /// Loads `rdf/graph.ttl` (`path`) into an in-memory store.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let store = Store::new().context("creating the in-memory RDF store")?;
        store
            .load_from_slice(RdfFormat::Turtle, &bytes)
            .with_context(|| format!("loading {}", path.display()))?;
        let mut prefixes: Vec<(String, String)> = dita2graph_core::rdf::PREFIXES
            .iter()
            .chain(EXTRA_PREFIXES)
            .map(|(p, iri)| (p.to_string(), iri.to_string()))
            .collect();
        if let Some(base) = data_base(&store)? {
            for (prefix, path) in [("node", "node/"), ("map", "map/"), ("key", "key/_/")] {
                prefixes.push((prefix.to_string(), format!("{base}{path}")));
            }
        }
        Ok(Self { store, prefixes })
    }

    /// Runs `query`, returning one page of its results as text: `limit`
    /// rows (capped at [`MAX_LIMIT`]) from `offset`, with the total count.
    pub fn query(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
        timeout: Duration,
    ) -> Result<String> {
        if query.chars().count() > MAX_QUERY_CHARS {
            bail!("the query is longer than {MAX_QUERY_CHARS} characters");
        }
        let limit = limit.clamp(1, MAX_LIMIT);
        let token = CancellationToken::new();
        // Built without Oxigraph's http-client feature: a SERVICE clause
        // has no handler to reach anything and fails.
        let mut evaluator = SparqlEvaluator::new().with_cancellation_token(token.clone());
        for (prefix, iri) in &self.prefixes {
            evaluator = evaluator
                .with_prefix(prefix, iri)
                .map_err(|e| anyhow!("prefix {prefix}: {e}"))?;
        }
        let prepared = evaluator.parse_query(query).map_err(|e| {
            anyhow!(
                "SPARQL syntax error (only SELECT, ASK, CONSTRUCT and DESCRIBE queries run): {e}"
            )
        })?;

        // A watchdog cancels the query when the timeout expires; dropping
        // `done` when the query ends releases it at once.
        let (done, watchdog) = mpsc::channel::<()>();
        let cancel = token.clone();
        let guard = std::thread::spawn(move || {
            if let Err(RecvTimeoutError::Timeout) = watchdog.recv_timeout(timeout) {
                cancel.cancel();
            }
        });
        let result = self.render(prepared.on_store(&self.store).execute(), limit, offset);
        drop(done);
        let _ = guard.join();
        result.map_err(|e| match e.downcast_ref::<QueryEvaluationError>() {
            Some(QueryEvaluationError::Cancelled) => anyhow!(
                "the query was stopped after {} s: narrow it (fewer unbound patterns, a FILTER on an id, a LIMIT)",
                timeout.as_secs()
            ),
            _ => e,
        })
    }

    fn render(
        &self,
        results: Result<QueryResults<'_>, QueryEvaluationError>,
        limit: usize,
        offset: usize,
    ) -> Result<String> {
        match results? {
            QueryResults::Boolean(value) => Ok(value.to_string()),
            QueryResults::Solutions(solutions) => {
                let variables: Vec<String> = solutions
                    .variables()
                    .iter()
                    .map(|v| format!("?{}", v.as_str()))
                    .collect();
                let stop_at = COUNT_CAP.max(offset.saturating_add(limit));
                let mut rows = Vec::new();
                let (mut total, mut truncated) = (0, false);
                for solution in solutions {
                    if total == stop_at {
                        truncated = true;
                        break;
                    }
                    let solution = solution?;
                    if total >= offset && rows.len() < limit {
                        let cells: Vec<String> = (0..variables.len())
                            .map(|i| solution.get(i).map(|t| self.term(t)).unwrap_or_default())
                            .collect();
                        rows.push(cells.join(" | "));
                    }
                    total += 1;
                }
                Ok(page(variables.join(" | "), rows, total, truncated, offset))
            }
            QueryResults::Graph(triples) => {
                let stop_at = COUNT_CAP.max(offset.saturating_add(limit));
                let mut rows = Vec::new();
                let (mut total, mut truncated) = (0, false);
                for triple in triples {
                    if total == stop_at {
                        truncated = true;
                        break;
                    }
                    let triple = triple?;
                    if total >= offset && rows.len() < limit {
                        rows.push(format!(
                            "{} {} {} .",
                            self.term(&triple.subject.into()),
                            self.iri(triple.predicate.as_str()),
                            self.term(&triple.object)
                        ));
                    }
                    total += 1;
                }
                Ok(page(
                    "subject predicate object".into(),
                    rows,
                    total,
                    truncated,
                    offset,
                ))
            }
        }
    }

    fn term(&self, term: &Term) -> String {
        match term {
            Term::NamedNode(node) => self.iri(node.as_str()),
            Term::BlankNode(node) => format!("_:{}", node.as_str()),
            Term::Literal(literal) => {
                let mut value: String = literal.value().chars().take(CELL_CHARS).collect();
                if literal.value().chars().count() > CELL_CHARS {
                    value.push('…');
                }
                let datatype = literal.datatype().as_str();
                // Numbers and booleans as SPARQL writes them: 102, not "102"^^xsd:integer.
                if ["integer", "decimal", "double", "boolean"]
                    .iter()
                    .any(|d| datatype == format!("http://www.w3.org/2001/XMLSchema#{d}"))
                {
                    return value;
                }
                let quoted = format!("{value:?}");
                if let Some(language) = literal.language() {
                    format!("{quoted}@{language}")
                } else if literal.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string" {
                    quoted
                } else {
                    format!("{quoted}^^{}", self.iri(literal.datatype().as_str()))
                }
            }
            #[allow(unreachable_patterns)]
            other => other.to_string(),
        }
    }

    /// `prefix:local` when a known namespace starts `iri` and the rest is
    /// a plain local name, else `<iri>`: what a query can paste back.
    fn iri(&self, iri: &str) -> String {
        self.prefixes
            .iter()
            .filter(|(_, ns)| iri.starts_with(ns.as_str()))
            .max_by_key(|(_, ns)| ns.len())
            .and_then(|(prefix, ns)| {
                let local = &iri[ns.len()..];
                plain_local_name(local).then(|| format!("{prefix}:{local}"))
            })
            .unwrap_or_else(|| format!("<{iri}>"))
    }
}

/// The text of one page: totals, the header, the rows, the next offset.
/// `truncated`: counting stopped at `total`, more results follow.
fn page(header: String, rows: Vec<String>, total: usize, truncated: bool, offset: usize) -> String {
    let count = if truncated {
        format!("more than {total} results (counting stopped there)")
    } else {
        format!("{total} result(s)")
    };
    if rows.is_empty() {
        return if total == 0 {
            "0 results".to_string()
        } else {
            format!("{count}; offset {offset} is past the last one")
        };
    }
    let end = offset + rows.len();
    let mut out = format!("{count}, showing {}-{end}.\n{header}\n", offset + 1);
    for row in &rows {
        let _ = writeln!(out, "{row}");
    }
    if end < total || truncated {
        let _ = write!(out, "(next page: offset={end})");
    }
    out.trim_end().to_string()
}

/// A local name SPARQL accepts after a prefix without escaping (a safe
/// subset: letters, digits, `_`, `-` and `.`, not ending with `.`).
fn plain_local_name(local: &str) -> bool {
    !local.starts_with(['-', '.'])
        && !local.ends_with('.')
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

/// The base of the data IRIs: the map's IRI is `<base>map/<id>`.
fn data_base(store: &Store) -> Result<Option<String>> {
    let query = format!(
        "SELECT ?map WHERE {{ ?map a <{}Map> }} LIMIT 1",
        dita2graph_core::rdf::NAMESPACE
    );
    let results = SparqlEvaluator::new()
        .parse_query(&query)
        .map_err(|e| anyhow!("{e}"))?
        .on_store(store)
        .execute()?;
    if let QueryResults::Solutions(mut solutions) = results
        && let Some(solution) = solutions.next()
        && let Some(Term::NamedNode(map)) = solution?.get("map")
        && let Some(index) = map.as_str().rfind("map/")
    {
        return Ok(Some(map.as_str()[..index].to_string()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use dita2graph_core::NormalizedNode;

    /// A small export: a map containing a task and a concept, a key, a
    /// URL the task links to, and a resource-only warehouse it reuses.
    fn graph() -> (tempfile::TempDir, SparqlGraph) {
        let nodes: Vec<NormalizedNode> = serde_json::from_value(serde_json::json!([
            { "type": "map", "id": "guide", "title": "Guide", "sourceFile": "guide.ditamap",
              "keyDefs": [{ "name": "brew", "url": "https://brew.sh", "sourceFile": "keys.ditamap" }],
              "links": [{ "relation": "contains", "target": "install" },
                        { "relation": "contains", "target": "setup" }] },
            { "type": "topic", "id": "install", "topicType": "task", "title": "Install",
              "sourceFile": "topics/install.dita", "keyrefs": ["brew"],
              "externalLinks": [{ "url": "https://brew.sh", "key": "brew" }],
              "links": [{ "relation": "requires", "target": "setup" },
                        { "relation": "generated-from", "target": "warehouse", "fragments": ["note"] }] },
            { "type": "topic", "id": "setup", "topicType": "concept", "title": "Setup",
              "sourceFile": "topics/setup.dita", "links": [] },
            { "type": "topic", "id": "warehouse", "topicType": "topic", "title": "Warehouse",
              "processingRole": "resource-only", "sourceFile": "topics/warehouse.dita", "links": [] }
        ]))
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        dita2graph_core::write_rdf(
            &nodes,
            dir.path(),
            Some("https://example.org/guide/"),
            "test",
            chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        )
        .unwrap();
        let graph = SparqlGraph::load(&dir.path().join("rdf/graph.ttl")).unwrap();
        (dir, graph)
    }

    #[test]
    fn select_uses_the_predeclared_prefixes_and_compacts_data_iris() {
        let (_dir, graph) = graph();
        let count = graph
            .query("SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }", 1, 0, TIMEOUT)
            .unwrap();
        assert!(count.starts_with("1 result(s)"), "{count}");
        let n: usize = count
            .lines()
            .last()
            .unwrap()
            .parse()
            .expect("a bare integer");
        assert!(n > 30, "{count}");
        let out = graph
            .query(
                "SELECT ?topic ?title WHERE { map:guide d2g:contains ?topic . ?topic dcterms:title ?title } ORDER BY ?title",
                DEFAULT_LIMIT,
                0,
                TIMEOUT,
            )
            .unwrap();
        assert_eq!(
            out,
            "2 result(s), showing 1-2.\n?topic | ?title\nnode:install | \"Install\"\nnode:setup | \"Setup\""
        );
    }

    #[test]
    fn a_path_query_follows_keys_to_the_url() {
        let (_dir, graph) = graph();
        let out = graph
            .query(
                "SELECT ?topic ?file WHERE { ?key d2g:keyTarget <https://brew.sh> ; d2g:sourceFile/d2g:path ?file . ?topic d2g:usesKey ?key }",
                DEFAULT_LIMIT,
                0,
                TIMEOUT,
            )
            .unwrap();
        assert!(out.ends_with("node:install | \"keys.ditamap\""), "{out}");
        let role = graph
            .query(
                "SELECT ?t WHERE { ?t d2g:processingRole \"resource-only\" }",
                DEFAULT_LIMIT,
                0,
                TIMEOUT,
            )
            .unwrap();
        assert!(role.ends_with("node:warehouse"), "{role}");
    }

    #[test]
    fn results_are_paginated_with_the_total_up_front() {
        let (_dir, graph) = graph();
        let query = "SELECT ?s WHERE { ?s a d2g:Node } ORDER BY ?s";
        let first = graph.query(query, 2, 0, TIMEOUT).unwrap();
        assert!(
            first.starts_with("4 result(s), showing 1-2.\n?s\n"),
            "{first}"
        );
        assert!(first.ends_with("(next page: offset=2)"), "{first}");
        let last = graph.query(query, 2, 2, TIMEOUT).unwrap();
        assert!(last.starts_with("4 result(s), showing 3-4."), "{last}");
        assert!(!last.contains("next page"), "{last}");
        let past = graph.query(query, 2, 9, TIMEOUT).unwrap();
        assert_eq!(past, "4 result(s); offset 9 is past the last one");
        // A huge offset or limit pages, without overflowing.
        let huge = graph.query(query, usize::MAX, usize::MAX, TIMEOUT).unwrap();
        assert!(huge.contains("past the last one"), "{huge}");
        let all = graph.query(query, usize::MAX, 1, TIMEOUT).unwrap();
        assert!(all.starts_with("4 result(s), showing 2-4."), "{all}");
    }

    #[test]
    fn ask_and_construct_run_too() {
        let (_dir, graph) = graph();
        assert_eq!(
            graph
                .query(
                    "ASK { node:install d2g:requires node:setup }",
                    10,
                    0,
                    TIMEOUT
                )
                .unwrap(),
            "true"
        );
        let out = graph
            .query(
                "CONSTRUCT { ?t d2g:linksTo ?url } WHERE { ?t d2g:linksTo ?url }",
                10,
                0,
                TIMEOUT,
            )
            .unwrap();
        assert!(
            out.ends_with("node:install d2g:linksTo <https://brew.sh> ."),
            "{out}"
        );
    }

    #[test]
    fn updates_and_syntax_errors_are_refused() {
        let (_dir, graph) = graph();
        let update = graph
            .query("INSERT DATA { node:a d2g:contains node:b }", 10, 0, TIMEOUT)
            .unwrap_err()
            .to_string();
        assert!(update.contains("SPARQL syntax error"), "{update}");
        let syntax = graph
            .query("SELECT WHERE {", 10, 0, TIMEOUT)
            .unwrap_err()
            .to_string();
        assert!(syntax.contains("SPARQL syntax error"), "{syntax}");
    }

    #[test]
    fn a_query_past_its_timeout_is_stopped() {
        let (_dir, graph) = graph();
        let err = graph
            .query(
                "SELECT * WHERE { ?a ?b ?c . ?d ?e ?f . ?g ?h ?i . ?j ?k ?l } ORDER BY ?a ?d ?g ?j",
                10,
                0,
                Duration::ZERO,
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("stopped after"), "{err}");
    }

    #[test]
    fn local_names_that_need_escaping_stay_full_iris() {
        assert!(plain_local_name("ID--topics-a.dita"));
        assert!(!plain_local_name("a/b"));
        assert!(!plain_local_name("li%4037"));
        assert!(!plain_local_name("end."));
    }
}
