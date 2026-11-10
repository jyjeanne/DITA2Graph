//! Golden test of the RDF export (`src/rdf.rs`): a normalized model that
//! uses every relation and node kind, exported on a fixed base and build
//! time, must give `tests/data/rdf-model.graph.ttl` byte for byte. CI also
//! validates that file against the SHACL shapes (`.github/workflows/
//! integration.yml`), so a change to the export shows up twice: as a diff
//! here, and as a shape violation there if it breaks the model.
//!
//! After an intended change, regenerate the file with
//! `UPDATE_GOLDEN=1 cargo test -p dita2graph-core --test rdf_export`
//! and review the diff.

use chrono::{TimeZone, Utc};
use dita2graph_core::{NormalizedNode, write_rdf};
use std::fs;
use std::path::Path;

#[test]
fn rdf_export_matches_the_golden_file() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let model = fs::read_to_string(data.join("rdf-model.json")).unwrap();
    let nodes: Vec<NormalizedNode> = serde_json::from_str(&model).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let summary = write_rdf(
        &nodes,
        dir.path(),
        Some("https://example.org/user-guide/"),
        "dita2graph-core/test",
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    )
    .unwrap();
    assert!(!summary.placeholder_base);
    let actual = fs::read_to_string(dir.path().join("rdf/graph.ttl")).unwrap();

    let golden = data.join("rdf-model.graph.ttl");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&golden, &actual).unwrap();
    }
    // A Windows checkout may have turned the golden file's LF into CRLF.
    let expected = fs::read_to_string(&golden)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "rdf/graph.ttl differs from {}; if the change is intended, rerun with \
         UPDATE_GOLDEN=1 and review the diff",
        golden.display()
    );
}
