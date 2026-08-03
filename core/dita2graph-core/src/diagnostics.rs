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

#[derive(Serialize)]
struct Diagnostic<'a> {
    id: &'static str,
    severity: Severity,
    message: &'a str,
}

/// Emits one diagnostic as a JSON line on stderr (§2.5).
pub fn emit(id: MessageId, message: &str) {
    let diagnostic = Diagnostic {
        id: id.0,
        severity: id.1,
        message,
    };
    if let Ok(line) = serde_json::to_string(&diagnostic) {
        let _ = writeln!(std::io::stderr(), "{line}");
    }
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
    }
}
