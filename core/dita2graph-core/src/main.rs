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
use dita2graph_core::{NormalizedNode, scan_bundle, write_bundle, write_rag_index};
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
        /// Backing store for the query index. `sqlite`/`rocksdb` are
        /// planned (§7 implementation stack) and not yet implemented in
        /// this scaffold; `none` is the only value that does anything
        /// today.
        #[arg(long, default_value = "none")]
        store: String,
        /// Whether to also write graph.json alongside the OKF bundle
        /// (§2.3's `args.dita2graph.emit-graph-json`). Accepts
        /// "true"/"false", matching the Ant property's own string
        /// values (`ExtractTask` forwards it verbatim).
        #[arg(long, default_value = "true")]
        emit_graph_json: String,
    },
    /// Validate an existing OKF bundle with `okf-validator` (§2.5, §6.4, §10).
    Validate {
        /// Path to the bundle directory (the `okf/` directory itself).
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Query the derived `graph.json` for a topic's relations (§3.4).
    /// A stand-in for the real SQLite/RocksDB-backed query index (§7),
    /// which is later Phase 2 work.
    Query {
        /// Output directory containing `graph.json` (i.e. what `--output`
        /// pointed at for `build`).
        #[arg(long = "store")]
        output_dir: PathBuf,
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
        } => run_build(input, output, store, emit_graph_json),
        Command::Validate { bundle } => run_validate(bundle),
        Command::Query {
            output_dir,
            topic,
            relation,
        } => run_query(output_dir, topic, relation),
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
) -> Result<ExitCode> {
    if store != "none" {
        eprintln!(
            "dita2graph-core: note: --store={store} is not implemented yet (see spec section 7); \
             no {store} index will be written."
        );
    }
    let emit_graph_json = parse_bool_arg(&emit_graph_json, "--emit-graph-json")?;

    let raw = fs::read_to_string(&input).with_context(|| format!("reading {}", input.display()))?;
    let nodes: Vec<NormalizedNode> =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", input.display()))?;

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

fn run_query(output_dir: PathBuf, topic: String, relation: Option<String>) -> Result<ExitCode> {
    let graph_path = output_dir.join("graph.json");
    let raw = fs::read_to_string(&graph_path).with_context(|| {
        format!(
            "reading {} (run `build` first, without --emit-graph-json=false)",
            graph_path.display()
        )
    })?;
    let graph: serde_json::Value = serde_json::from_str(&raw)?;

    let edges = graph["edges"].as_array().cloned().unwrap_or_default();
    let mut found = false;
    for edge in &edges {
        let from = edge["from"].as_str().unwrap_or_default();
        let edge_relation = edge["relation"].as_str().unwrap_or_default();
        if from != topic {
            continue;
        }
        if let Some(want) = &relation
            && edge_relation != want
        {
            continue;
        }
        found = true;
        println!(
            "{topic} --{edge_relation}--> {}",
            edge["to"].as_str().unwrap_or_default()
        );
    }

    if !found {
        eprintln!("dita2graph-core: no matching edges for topic `{topic}`");
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}
