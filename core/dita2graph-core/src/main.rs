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
use dita2graph_core::{NormalizedNode, scan_bundle, write_bundle};
use std::fs;
use std::path::PathBuf;
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
        /// Output directory; `okf/` and `graph.json` are written under it (§2.4).
        #[arg(long)]
        output: PathBuf,
        /// Backing store for the query index. `sqlite`/`rocksdb` are
        /// planned (§7 implementation stack) and not yet implemented in
        /// this scaffold; `none` is the only value that does anything
        /// today (`graph.json` is always written regardless).
        #[arg(long, default_value = "none")]
        store: String,
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
        } => run_build(input, output, store),
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

fn run_build(input: PathBuf, output: PathBuf, store: String) -> Result<ExitCode> {
    if store != "none" {
        eprintln!(
            "dita2graph-core: note: --store={store} is not implemented yet (see spec section 7); \
             writing graph.json only, no {store} index."
        );
    }

    let raw = fs::read_to_string(&input).with_context(|| format!("reading {}", input.display()))?;
    let nodes: Vec<NormalizedNode> =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", input.display()))?;

    let summary = write_bundle(&nodes, &output, Utc::now())?;
    println!(
        "wrote {} topics, {} maps, {} edges to {}",
        summary.topics_written,
        summary.maps_written,
        summary.edges_written,
        output.join("okf").display()
    );

    // A bundle that fails validation isn't a complete build (§2.5): run
    // the same check `validate` does before declaring success.
    validate_and_report(&output.join("okf"))
}

fn run_validate(bundle: PathBuf) -> Result<ExitCode> {
    validate_and_report(&bundle)
}

fn validate_and_report(bundle: &std::path::Path) -> Result<ExitCode> {
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
        return Ok(ExitCode::from(1));
    }

    // A bundle can be format-valid per `okf-validator` and still leak a
    // secret into generated prose (§6.4); that's a build-breaking error,
    // not a warning, so it's checked separately and still fails the build.
    let secret_findings = scan_bundle(bundle)?;
    if !secret_findings.is_empty() {
        for finding in &secret_findings {
            println!(
                "Error {}: possible secret leak ({})",
                finding.file, finding.pattern
            );
        }
        diagnostics::emit(
            POSSIBLE_SECRET_LEAK,
            &format!(
                "{} file(s) in {} match a high-confidence secret pattern",
                secret_findings.len(),
                bundle.display()
            ),
        );
        return Ok(ExitCode::from(1));
    }

    println!("bundle OK: {}", bundle.display());
    Ok(ExitCode::SUCCESS)
}

fn run_query(output_dir: PathBuf, topic: String, relation: Option<String>) -> Result<ExitCode> {
    let graph_path = output_dir.join("graph.json");
    let raw = fs::read_to_string(&graph_path)
        .with_context(|| format!("reading {} (run `build` first)", graph_path.display()))?;
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
