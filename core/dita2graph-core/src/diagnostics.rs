//! Structured diagnostics matching the `DITA2GRAPHnnnX` message catalog
//! in `docs/plugin-specification.md` §2.5. When `dita2graph-core` runs
//! standalone (not driven by the DITA-OT Java plugin, which has its own
//! `cfg/messages.xml`-based logger), diagnostics are emitted as one JSON
//! object per line on stderr, so CI can parse them without depending on
//! DITA-OT's own log format.

use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Fatal,
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy)]
pub struct MessageId(pub &'static str, pub Severity);

/// Unresolved `keyref`/`conkeyref` during extraction. Fatal at the Java
/// plugin layer (§2.2); the Rust core only observes an already-normalized
/// model, so it can't itself hit this one, but the ID is shared here so
/// Rust-side error text can reference it consistently.
pub const UNRESOLVED_KEYREF: MessageId = MessageId("DITA2GRAPH001E", Severity::Error);
/// Ambiguous relation inference; the lower-confidence edge is dropped, not guessed.
pub const AMBIGUOUS_RELATION: MessageId = MessageId("DITA2GRAPH010W", Severity::Warning);
/// Topic skipped because of `status="draft"` and `include-drafts=false`.
pub const DRAFT_TOPIC_SKIPPED: MessageId = MessageId("DITA2GRAPH020I", Severity::Info);
/// Generated OKF concept failed `okf-validator` conformance.
pub const BUNDLE_VALIDATION_FAILED: MessageId = MessageId("DITA2GRAPH030E", Severity::Error);
/// Topic has no resolvable `type` mapping; emitted as a generic concept.
pub const UNKNOWN_TOPIC_TYPE: MessageId = MessageId("DITA2GRAPH040W", Severity::Warning);
/// A generated concept matches a high-confidence secret pattern (§6.4).
pub const POSSIBLE_SECRET_LEAK: MessageId = MessageId("DITA2GRAPH050E", Severity::Error);
/// The RDF export breaks a `sh:Violation` rule of the d2g shapes: the build fails.
pub const SHACL_VIOLATION: MessageId = MessageId("DITA2GRAPH080E", Severity::Error);
/// The RDF export breaks a `sh:Warning` rule of the d2g shapes: reported only.
pub const SHACL_WARNING: MessageId = MessageId("DITA2GRAPH081W", Severity::Warning);
/// The RDF export breaks a `sh:Info` rule of the d2g shapes: reported only.
pub const SHACL_NOTE: MessageId = MessageId("DITA2GRAPH082I", Severity::Info);
/// The RDF export has no `--rdf-base`: its IRIs sit under the reserved
/// `https://dita2graph.invalid/` domain and are not meant to be published.
pub const PLACEHOLDER_RDF_BASE: MessageId = MessageId("DITA2GRAPH083W", Severity::Warning);

/// Every message of the catalog. The DITA-OT plugin's `CoreDiagnostic`
/// parses a line of each (`tests/diagnostics.rs` writes them).
pub const CATALOG: [MessageId; 10] = [
    UNRESOLVED_KEYREF,
    AMBIGUOUS_RELATION,
    DRAFT_TOPIC_SKIPPED,
    BUNDLE_VALIDATION_FAILED,
    UNKNOWN_TOPIC_TYPE,
    POSSIBLE_SECRET_LEAK,
    SHACL_VIOLATION,
    SHACL_WARNING,
    SHACL_NOTE,
    PLACEHOLDER_RDF_BASE,
];

#[derive(Serialize)]
struct Diagnostic<'a> {
    id: &'static str,
    severity: Severity,
    message: &'a str,
}

/// Emits one diagnostic as a JSON line on stderr (§2.5).
pub fn emit(id: MessageId, message: &str) {
    let _ = writeln!(std::io::stderr(), "{}", line(id, message));
}

/// The JSON line of one diagnostic,
/// `{"id":"DITA2GRAPH081W","severity":"warning","message":"..."}`: the
/// shape the DITA-OT plugin's `CoreDiagnostic` parses, so its fields keep
/// this order.
pub fn line(id: MessageId, message: &str) -> String {
    let diagnostic = Diagnostic {
        id: id.0,
        severity: id.1,
        message,
    };
    serde_json::to_string(&diagnostic).expect("a diagnostic serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_ids_match_the_spec_catalog() {
        assert_eq!(UNRESOLVED_KEYREF.0, "DITA2GRAPH001E");
        assert_eq!(AMBIGUOUS_RELATION.0, "DITA2GRAPH010W");
        assert_eq!(DRAFT_TOPIC_SKIPPED.0, "DITA2GRAPH020I");
        assert_eq!(BUNDLE_VALIDATION_FAILED.0, "DITA2GRAPH030E");
        assert_eq!(UNKNOWN_TOPIC_TYPE.0, "DITA2GRAPH040W");
        assert_eq!(POSSIBLE_SECRET_LEAK.0, "DITA2GRAPH050E");
        assert_eq!(SHACL_VIOLATION.0, "DITA2GRAPH080E");
        assert_eq!(SHACL_WARNING.0, "DITA2GRAPH081W");
        assert_eq!(SHACL_NOTE.0, "DITA2GRAPH082I");
        assert_eq!(PLACEHOLDER_RDF_BASE.0, "DITA2GRAPH083W");
    }

    #[test]
    fn each_id_ends_with_the_letter_of_its_severity() {
        for MessageId(id, severity) in CATALOG {
            let letter = match severity {
                Severity::Fatal => 'F',
                Severity::Error => 'E',
                Severity::Warning => 'W',
                Severity::Info => 'I',
            };
            assert!(id.ends_with(letter), "{id} is {severity:?}");
        }
    }

    #[test]
    fn the_catalog_lists_each_message_once() {
        let ids: std::collections::HashSet<_> = CATALOG.iter().map(|m| m.0).collect();
        assert_eq!(ids.len(), CATALOG.len());
    }
}
