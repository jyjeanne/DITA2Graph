//! `dita2graph-core` CLI (§3.4): the standalone binary, independent of
//! DITA-OT, used directly in this Phase 0/1 scaffold (there is no
//! working Java→Rust IPC yet — see `docs/dev/phase-0-findings.md`) and
//! eventually invoked by `bin/dita2graph`/`build.xml` (§2.1).
//!
//! Exit codes follow §2.5: `0` success, `1` validation failure, `2`
//! internal error.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use dita2graph_core::diagnostics::{self, BUNDLE_VALIDATION_FAILED, POSSIBLE_SECRET_LEAK};
use dita2graph_core::{
    Embedder, NormalizedNode, infer_applies_to, infer_related_to, query_sqlite_store, scan_bundle,
    write_bundle, write_embeddings_index, write_mcp_config, write_rag_index, write_sqlite_store,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "dita2graph-core", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build an OKF bundle from a normalized DITA model (§3.2), then
    /// validate it before declaring success.
    Build {
        /// Path to a JSON file containing an array of normalized nodes.
        #[arg(long)]
        input: PathBuf,
        /// Output directory; `okf/`, `graph.json`, and `rag/` (§13.1) are
        /// written under it (§2.4).
        #[arg(long)]
        output: PathBuf,
        /// Backing store for the query index (§7 implementation stack).
        /// `sqlite` writes `<output>/graph.db`, a SQLite mirror of
        /// `graph.json`'s nodes/edges for fast indexed `query` lookups
        /// on a real corpus (`src/store.rs`); `rocksdb` is still planned,
        /// not implemented; `none` (default) writes no index -- `query`
        /// falls back to reading `graph.json` directly either way.
        #[arg(long, default_value = "none")]
        store: String,
        /// Whether to also write graph.json alongside the OKF bundle
        /// (§2.3's `args.dita2graph.emit-graph-json`). Accepts
        /// "true"/"false", matching the Ant property's own string
        /// values (`ExtractTask` forwards it verbatim).
        #[arg(long, default_value = "true")]
        emit_graph_json: String,
        /// Whether to also write mcp/mcp-server.toml (§2.3's
        /// `args.dita2graph.mcp`), a real config `dita2graph-mcp
        /// --config` can read. Accepts "true"/"false".
        #[arg(long, default_value = "false")]
        mcp: String,
        /// Path to an ONNX sentence-embedding model (§13.1's node-level
        /// embeddings). Optional -- when given, must be paired with
        /// `--embedding-tokenizer`; when omitted, no `rag/embeddings.jsonl`
        /// is written and `search_content` stays keyword-only, unchanged
        /// from today. Requires `ORT_DYLIB_PATH` to point at a real ONNX
        /// Runtime shared library at run time (`src/embeddings.rs`).
        #[arg(long, requires = "embedding_tokenizer")]
        embedding_model: Option<PathBuf>,
        /// Path to the tokenizer.json matching `--embedding-model`.
        #[arg(long, requires = "embedding_model")]
        embedding_tokenizer: Option<PathBuf>,
    },
    /// Validate an existing OKF bundle with `okf-validator` (§2.5, §6.4, §10).
    Validate {
        /// Path to the bundle directory (the `okf/` directory itself).
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Query a topic's relations (§3.4), from either `graph.json` or a
    /// `graph.db` written by `build --store sqlite` -- RocksDB-backed
    /// storage remains later Phase 6+ work (§7).
    Query {
        /// Either a bundle output directory containing `graph.json`
        /// (i.e. what `--output` pointed at for `build`) or a direct
        /// path to a `graph.db` written by `build --store sqlite` --
        /// distinguished by whether the path is itself an existing file.
        #[arg(long = "store")]
        store: PathBuf,
        #[arg(long)]
        topic: String,
        #[arg(long)]
        relation: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Build {
            input,
            output,
            store,
            emit_graph_json,
            mcp,
            embedding_model,
            embedding_tokenizer,
        } => run_build(
            input,
            output,
            store,
            emit_graph_json,
            mcp,
            embedding_model,
            embedding_tokenizer,
        ),
        Command::Validate { bundle } => run_validate(bundle),
        Command::Query {
            store,
            topic,
            relation,
        } => run_query(store, topic, relation),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("dita2graph-core: internal error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run_build(
    input: PathBuf,
    output: PathBuf,
    store: String,
    emit_graph_json: String,
    mcp: String,
    embedding_model: Option<PathBuf>,
    embedding_tokenizer: Option<PathBuf>,
) -> Result<ExitCode> {
    match store.as_str() {
        "none" | "sqlite" => {}
        other => {
            eprintln!(
                "dita2graph-core: note: --store={other} is not implemented yet (see spec \
                 section 7); no {other} index will be written."
            );
        }
    }
    let emit_graph_json = parse_bool_arg(&emit_graph_json, "--emit-graph-json")?;
    let mcp = parse_bool_arg(&mcp, "--mcp")?;

    let raw = fs::read_to_string(&input).with_context(|| format!("reading {}", input.display()))?;
    let mut nodes: Vec<NormalizedNode> =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", input.display()))?;

    // Relation inference (§3.3) augments the model in place, before
    // either the OKF bundle or graph.json is written, so inferred edges
    // show up in both rather than needing a separate pass. applies-to
    // runs first: it's the higher-confidence, directional, type-scoped
    // signal, so it gets first claim on a pair before the broader,
    // symmetric related-to sweep considers it (relations.rs).
    let applies_to_inferred = infer_applies_to(&mut nodes);
    if applies_to_inferred > 0 {
        println!("inferred {applies_to_inferred} applies-to edge(s)");
    }
    let related_to_inferred = infer_related_to(&mut nodes);
    if related_to_inferred > 0 {
        println!("inferred {related_to_inferred} related-to edge(s)");
    }

    // Single pass over `nodes` feeding two correlated outputs (§13.1):
    // the OKF graph and the RAG content index share the same in-memory
    // normalized model rather than each re-deriving it.
    let generated_at = Utc::now();

    let summary = write_bundle(&nodes, &output, generated_at, emit_graph_json)?;
    println!(
        "wrote {} topics, {} maps, {} edges to {}",
        summary.topics_written,
        summary.maps_written,
        summary.edges_written,
        output.join("okf").display()
    );

    let rag_summary = write_rag_index(&nodes, &output, generated_at)?;
    println!(
        "wrote {} chunk(s) to {}",
        rag_summary.chunks_written,
        output.join("rag").display()
    );

    if store == "sqlite" {
        let store_summary = write_sqlite_store(&nodes, &output)?;
        println!(
            "wrote {} node(s), {} edge(s) to {}",
            store_summary.nodes_written,
            store_summary.edges_written,
            output.join("graph.db").display()
        );
    } else {
        // `dita2graph-mcp`'s `BundleReader` prefers `graph.db` over
        // `graph.json` whenever the file exists (`bundle.rs::open`), on
        // the assumption that its presence means the most recent build
        // asked for it. A leftover `graph.db` from an *earlier* build
        // that used `--store sqlite`, followed by a later rebuild that
        // didn't, would otherwise silently violate that assumption --
        // the MCP server would keep serving a stale graph index forever,
        // never touching the freshly rewritten graph.json. Removing it
        // here keeps "graph.db exists" a reliable signal for "this
        // build's own --store sqlite", the same discipline
        // `write_sqlite_store` already applies *within* a single sqlite
        // build (always starting from a clean file).
        let stale_db = output.join("graph.db");
        if stale_db.exists() {
            fs::remove_file(&stale_db)
                .with_context(|| format!("removing stale {}", stale_db.display()))?;
            println!(
                "removed stale {} (this build didn't request --store sqlite)",
                stale_db.display()
            );
        }
    }

    if let (Some(model_path), Some(tokenizer_path)) = (&embedding_model, &embedding_tokenizer) {
        let model_name = model_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| model_path.display().to_string());
        let embedder = Embedder::load(model_path, tokenizer_path).with_context(|| {
            format!(
                "loading embedding model {} / tokenizer {}",
                model_path.display(),
                tokenizer_path.display()
            )
        })?;
        let embedding_summary = write_embeddings_index(&nodes, &output, &embedder, &model_name)?;
        println!(
            "wrote {} embedding(s) (dim {}) to {}",
            embedding_summary.embeddings_written,
            embedding_summary.dim,
            output.join("rag/embeddings.jsonl").display()
        );
    }

    if mcp {
        write_mcp_config(&output)?;
        println!("wrote mcp config to {}", output.join("mcp").display());
    }

    // A bundle that fails validation isn't a complete build (§2.5): run
    // the same okf-validator + secret-scan checks `validate` does on
    // okf/, plus a secret scan over rag/ -- okf-validator only knows
    // okf/'s format, so rag/ gets its own scan, not folded into
    // validate_and_report (§6.4, §13.1).
    let okf_ok = validate_and_report(&output.join("okf"))?;
    let rag_ok = scan_rag_and_report(&output.join("rag"))?;
    Ok(if okf_ok && rag_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Parses a `"true"`/`"false"` CLI value, matching the Ant property
/// string convention `ExtractTask` forwards these args in, rather than
/// clap's own flag/switch parsing (§2.3).
fn parse_bool_arg(value: &str, flag: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => anyhow::bail!("{flag}: expected \"true\" or \"false\", got {other:?}"),
    }
}

fn run_validate(bundle: PathBuf) -> Result<ExitCode> {
    Ok(if validate_and_report(&bundle)? {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Runs `okf-validator` conformance checks plus the secret scan (§6.4)
/// against an `okf/` bundle directory, printing results as it goes.
/// Returns whether the bundle passed both checks.
fn validate_and_report(bundle: &Path) -> Result<bool> {
    let report = okf_validator::validate_bundle(bundle)
        .with_context(|| format!("validating {}", bundle.display()))?;
    for issue in &report.issues {
        println!("{:?} {}: {}", issue.severity, issue.file, issue.message);
    }
    if report.has_errors() {
        diagnostics::emit(
            BUNDLE_VALIDATION_FAILED,
            &format!(
                "{} validation issue(s) found in {}",
                report.issues.len(),
                bundle.display()
            ),
        );
        return Ok(false);
    }

    // A bundle can be format-valid per `okf-validator` and still leak a
    // secret into generated prose (§6.4); that's a build-breaking error,
    // not a warning, so it's checked separately and still fails the build.
    if !scan_and_report(bundle)? {
        return Ok(false);
    }

    println!("bundle OK: {}", bundle.display());
    Ok(true)
}

/// Runs just the secret scan (§6.4) against `dir`, printing results.
/// Used both by `validate_and_report` (for `okf/`) and directly (for
/// `rag/`, which isn't OKF-conformant format so `okf-validator` doesn't
/// apply to it, §13.1).
fn scan_and_report(dir: &Path) -> Result<bool> {
    let findings = scan_bundle(dir)?;
    if findings.is_empty() {
        return Ok(true);
    }
    for finding in &findings {
        println!(
            "Error {}: possible secret leak ({})",
            finding.file, finding.pattern
        );
    }
    diagnostics::emit(
        POSSIBLE_SECRET_LEAK,
        &format!(
            "{} file(s) in {} match a high-confidence secret pattern",
            findings.len(),
            dir.display()
        ),
    );
    Ok(false)
}

fn scan_rag_and_report(rag_dir: &Path) -> Result<bool> {
    let ok = scan_and_report(rag_dir)?;
    if ok {
        println!("rag index OK: {}", rag_dir.display());
    }
    Ok(ok)
}

/// `--store` accepts either a `graph.db` file directly (the spec's §3.4
/// example: `query --store output/graph.db ...`), read via
/// [`query_sqlite_store`], or a bundle output directory containing
/// `graph.json` (the original, still-default behavior, unchanged) --
/// distinguished by whether the given path is itself an existing file,
/// not by extension, so an existing invocation passing a directory
/// behaves exactly as before.
fn run_query(store: PathBuf, topic: String, relation: Option<String>) -> Result<ExitCode> {
    let edges = if store.is_file() {
        query_sqlite_store(&store, &topic, relation.as_deref())?
    } else {
        query_graph_json(&store, &topic, relation.as_deref())?
    };

    if edges.is_empty() {
        eprintln!("dita2graph-core: no matching edges for topic `{topic}`");
        return Ok(ExitCode::FAILURE);
    }
    for (edge_relation, to) in &edges {
        println!("{topic} --{edge_relation}--> {to}");
    }
    Ok(ExitCode::SUCCESS)
}

fn query_graph_json(
    output_dir: &Path,
    topic: &str,
    relation: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let graph_path = output_dir.join("graph.json");
    let raw = fs::read_to_string(&graph_path).with_context(|| {
        format!(
            "reading {} (run `build` first, without --emit-graph-json=false)",
            graph_path.display()
        )
    })?;
    let graph: serde_json::Value = serde_json::from_str(&raw)?;

    let edges = graph["edges"].as_array().cloned().unwrap_or_default();
    let mut matched = Vec::new();
    for edge in &edges {
        let from = edge["from"].as_str().unwrap_or_default();
        let edge_relation = edge["relation"].as_str().unwrap_or_default();
        if from != topic {
            continue;
        }
        if let Some(want) = relation
            && edge_relation != want
        {
            continue;
        }
        matched.push((
            edge_relation.to_string(),
            edge["to"].as_str().unwrap_or_default().to_string(),
        ));
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dita2graph_core::{Link, NormalizedMap, NormalizedTopic, Relation, TopicType};

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
                        target: "installing-product-prereqs".into(),
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
            NormalizedNode::Topic(NormalizedTopic {
                id: "installing-product-prereqs".into(),
                topic_type: TopicType::Concept,
                title: "Prerequisites".into(),
                shortdesc: None,
                body: None,
                audience: vec![],
                product: vec![],
                keys: vec![],
                uicontrols: vec![],
                cmd_uicontrols: vec![],
                source_file: "topics/installing-product-prereqs.dita".into(),
                links: vec![],
            }),
        ]
    }

    /// `dita2graph-mcp`'s `BundleReader` treats `graph.db`'s mere
    /// existence as "the most recent build asked for --store sqlite"
    /// (`mcp/dita2graph-mcp/src/bundle.rs::open`) -- a `graph.db` left
    /// over from an *earlier* sqlite build, still sitting next to a
    /// freshly rewritten `graph.json` from a later non-sqlite build,
    /// would violate that and serve a silently stale index forever.
    /// `run_build` must remove it when `--store` isn't `sqlite`.
    #[test]
    fn run_build_removes_a_stale_graph_db_when_store_reverts_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let input_path = dir.path().join("normalized-model.json");
        fs::write(&input_path, serde_json::to_string(&sample_nodes()).unwrap()).unwrap();
        let output = dir.path().join("out");

        let code = run_build(
            input_path.clone(),
            output.clone(),
            "sqlite".to_string(),
            "true".to_string(),
            "false".to_string(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(output.join("graph.db").exists());

        let code = run_build(
            input_path,
            output.clone(),
            "none".to_string(),
            "true".to_string(),
            "false".to_string(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            !output.join("graph.db").exists(),
            "a rebuild without --store sqlite must remove the earlier build's graph.db"
        );
    }

    /// The real bug risk in having two independent read paths
    /// (`query_graph_json`, `query_sqlite_store`) for the same
    /// `--store` flag: they silently drift and answer differently for
    /// the same bundle. Builds both a `graph.json` and a `graph.db` from
    /// the identical in-memory model (exactly what `run_build` does when
    /// `--store sqlite` is given -- both are always written together,
    /// never just one) and asserts unscoped and relation-scoped queries
    /// return the same edge set from either, order aside (SQL has no
    /// row-order guarantee without `ORDER BY`, and neither backend ever
    /// promised one).
    #[test]
    fn graph_json_and_sqlite_store_answer_the_same_query_identically() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        dita2graph_core::write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
        dita2graph_core::write_sqlite_store(&nodes, dir.path()).unwrap();

        let mut from_json = query_graph_json(dir.path(), "installing-product", None).unwrap();
        let mut from_sqlite =
            query_sqlite_store(&dir.path().join("graph.db"), "installing-product", None).unwrap();
        from_json.sort();
        from_sqlite.sort();
        assert_eq!(from_json, from_sqlite);
        assert_eq!(
            from_json,
            vec![
                (
                    "references".to_string(),
                    "installing-product-prereqs".to_string()
                ),
                ("requires".to_string(), "configuration".to_string()),
            ]
        );

        let mut from_json_scoped =
            query_graph_json(dir.path(), "installing-product", Some("requires")).unwrap();
        let mut from_sqlite_scoped = query_sqlite_store(
            &dir.path().join("graph.db"),
            "installing-product",
            Some("requires"),
        )
        .unwrap();
        from_json_scoped.sort();
        from_sqlite_scoped.sort();
        assert_eq!(from_json_scoped, from_sqlite_scoped);
        assert_eq!(
            from_json_scoped,
            vec![("requires".to_string(), "configuration".to_string())]
        );
    }

    /// `run_query`'s `store.is_file()` dispatch is the only thing
    /// deciding which backend answers a query -- regression coverage for
    /// that specific branch, independent of the two backends' own
    /// correctness (covered above and in `store.rs`).
    #[test]
    fn run_query_dispatches_to_sqlite_only_when_store_is_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let nodes = sample_nodes();
        dita2graph_core::write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
        dita2graph_core::write_sqlite_store(&nodes, dir.path()).unwrap();

        // A directory: graph.json path, unchanged from before --store
        // sqlite existed.
        let code = run_query(
            dir.path().to_path_buf(),
            "installing-product".to_string(),
            None,
        )
        .unwrap();
        assert_eq!(code, ExitCode::SUCCESS);

        // The graph.db file directly: sqlite path.
        let code = run_query(
            dir.path().join("graph.db"),
            "installing-product".to_string(),
            None,
        )
        .unwrap();
        assert_eq!(code, ExitCode::SUCCESS);

        // Neither exists at all: still a clean "no matches" failure
        // exit, not a panic or an internal error.
        let code = run_query(dir.path().to_path_buf(), "no-such-topic".to_string(), None).unwrap();
        assert_eq!(code, ExitCode::FAILURE);
    }
}
