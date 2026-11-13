//! The `dita2graph-core` binary as the DITA-OT plugin runs it
//! (`ExtractTask.java`): `build` on the model the plugin writes
//! (`tests/data/java-model.json`), its exit codes (0 success, 1 a bundle
//! that fails validation, 2 an internal error: §2.5), which the task turns
//! into a failed DITA-OT build, and the diagnostic lines it writes on
//! stderr, which the task logs at their own level.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn core(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dita2graph-core"))
        .args(args)
        .env("SOURCE_DATE_EPOCH", "0")
        .output()
        .unwrap()
}

fn java_model() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/java-model.json")
}

fn build(input: &Path, output: &Path, extra: &[&str]) -> Output {
    let mut args = vec![
        "build",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    core(&args)
}

/// The diagnostics on stderr, as `CoreDiagnostic.java` reads them: whole
/// lines whose fields come in the order id, severity, message.
fn diagnostics(output: &Output) -> Vec<(String, String)> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 3, "{line}");
            assert!(line.starts_with("{\"id\":\"DITA2GRAPH"), "{line}");
            let severity = line.find(",\"severity\":").expect(line);
            assert!(line[severity..].contains(",\"message\":"), "{line}");
            (
                value["id"].as_str().unwrap().to_string(),
                value["severity"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn version_is_the_workspace_version() {
    let output = core(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("dita2graph-core ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn the_plugin_model_builds_every_output_and_validates() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path();
    let output = build(
        &java_model(),
        out,
        &[
            "--store",
            "sqlite",
            "--mcp",
            "true",
            "--emit-rdf",
            "true",
            "--rdf-base",
            "https://example.org/user-guide/",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for file in [
        "okf/maps/user-guide.md",
        "okf/topics/installing.md",
        "graph.json",
        "graph.db",
        "rag/chunks.jsonl",
        "rdf/graph.ttl",
        "mcp/mcp-server.toml",
    ] {
        assert!(out.join(file).is_file(), "{file} missing");
    }
    assert!(
        !diagnostics(&output)
            .iter()
            .any(|(_, severity)| severity == "error"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let okf = out.join("okf");
    let validate = core(&["validate", "--bundle", okf.to_str().unwrap()]);
    assert_eq!(validate.status.code(), Some(0));

    let query = core(&[
        "query",
        "--store",
        out.join("graph.db").to_str().unwrap(),
        "--topic",
        "installing",
        "--relation",
        "requires",
    ]);
    assert_eq!(query.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&query.stdout).trim(),
        "installing --requires--> prerequisites"
    );
}

#[test]
fn an_rdf_export_without_a_base_warns_with_083w() {
    let dir = tempfile::tempdir().unwrap();
    let output = build(&java_model(), dir.path(), &["--emit-rdf", "true"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(
        diagnostics(&output).contains(&("DITA2GRAPH083W".into(), "warning".into())),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_secret_in_the_content_fails_the_build_with_exit_code_1() {
    let dir = tempfile::tempdir().unwrap();
    let mut model: Value =
        serde_json::from_str(&fs::read_to_string(java_model()).unwrap()).unwrap();
    model[2]["body"] = Value::from("Set the key to AKIAIOSFODNN7EXAMPLE.");
    let input = dir.path().join("model.json");
    fs::write(&input, model.to_string()).unwrap();

    let output = build(&input, &dir.path().join("out"), &[]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        diagnostics(&output).contains(&("DITA2GRAPH050E".into(), "error".into())),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_bundle_that_breaks_the_okf_format_fails_validation_with_exit_code_1() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    assert!(build(&java_model(), &out, &[]).status.success());
    // A concept file without its frontmatter.
    fs::write(out.join("okf/topics/settings.md"), "# Settings\n").unwrap();

    let okf = out.join("okf");
    let output = core(&["validate", "--bundle", okf.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        diagnostics(&output).contains(&("DITA2GRAPH030E".into(), "error".into())),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_model_that_is_not_json_is_an_internal_error_with_exit_code_2() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("model.json");
    fs::write(&input, "not json").unwrap();

    let output = build(&input, &dir.path().join("out"), &[]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("internal error") && stderr.contains("parsing"),
        "{stderr}"
    );
    assert!(!dir.path().join("out/okf").exists(), "nothing written");
}

#[test]
fn a_flag_value_other_than_true_or_false_is_an_internal_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = build(&java_model(), dir.path(), &["--emit-rdf", "yes"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--emit-rdf"));
}
