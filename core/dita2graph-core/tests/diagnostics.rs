//! Golden test of the diagnostic lines `dita2graph-core` writes
//! (`src/diagnostics.rs`). The DITA-OT plugin reads them from the core's
//! output and logs each at its own level (`CoreDiagnostic.java`), and its
//! `CoreDiagnosticTest` parses this same file: a change to the line format
//! on either side fails one of the two tests.
//!
//! The file holds one line per message of the catalog, then lines whose
//! messages carry the characters JSON has to escape, and some it doesn't.
//! After an intended change, regenerate it with
//! `UPDATE_GOLDEN=1 cargo test -p dita2graph-core --test diagnostics`
//! and review the diff.

use dita2graph_core::diagnostics::{
    self, CATALOG, PLACEHOLDER_RDF_BASE, SHACL_NOTE, SHACL_VIOLATION, SHACL_WARNING,
};
use std::fs;
use std::path::Path;

/// Messages `CoreDiagnosticTest.java` expects back, in this order.
const MESSAGES: [(diagnostics::MessageId, &str); 4] = [
    (
        SHACL_WARNING,
        "PublishedTopicShape on https://ex.org/node/a: \"Published\" topic é",
    ),
    (SHACL_VIOLATION, "C:\\docs\\a.dita loops back\tto itself"),
    (SHACL_NOTE, "first line\nsecond line"),
    (PLACEHOLDER_RDF_BASE, "日本語 🚀 \u{1}"),
];

#[test]
fn diagnostic_lines_match_the_golden_file() {
    let mut actual = String::new();
    for id in CATALOG {
        actual.push_str(&diagnostics::line(id, "sample message"));
        actual.push('\n');
    }
    for (id, message) in MESSAGES {
        actual.push_str(&diagnostics::line(id, message));
        actual.push('\n');
    }

    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/diagnostics.jsonl");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&golden, &actual).unwrap();
    }
    // A Windows checkout may have turned the golden file's LF into CRLF.
    let expected = fs::read_to_string(&golden)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "diagnostic lines differ from {}; if the change is intended, rerun with \
         UPDATE_GOLDEN=1 and review the diff",
        golden.display()
    );
}

#[test]
fn every_line_is_one_line_of_output() {
    for (id, message) in MESSAGES {
        assert!(!diagnostics::line(id, message).contains('\n'));
    }
}
