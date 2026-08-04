//! `dita2graph-core`: normalizes DITA-OT's resolved model (§3.2) into an
//! OKF v0.2 knowledge bundle (§4), per `docs/plugin-specification.md`.
//!
//! This crate implements Phase 1/2 of the roadmap in §12: the normalized
//! model contract, the OKF bundle writer, and the diagnostics catalog.
//! Relation *inference* (deriving edges DITA doesn't state explicitly,
//! §3.3) is partially implemented: `related-to` (shared `product`
//! values, `relations.rs`) is real; `applies-to` and `generated-from`
//! remain unimplemented, since they need DITA extraction this scaffold
//! doesn't do yet (`<uicontrol>` scanning, `conref`/`conkeyref`
//! provenance). The SQLite/RocksDB query index is later Phase 2 work and
//! is not yet implemented — `graph.json` (a flattened, derived view) is
//! written today and is enough for the `query` CLI subcommand.

pub mod diagnostics;
pub mod mcp_config;
pub mod model;
pub mod okf;
pub mod rag;
pub mod relations;
pub mod secrets;

pub use mcp_config::write_mcp_config;
pub use model::{Link, NormalizedMap, NormalizedNode, NormalizedTopic, Relation, TopicType};
pub use okf::{BundleSummary, write_bundle};
pub use rag::{RagSummary, write_rag_index};
pub use relations::infer_related_to;
pub use secrets::{SecretFinding, scan_bundle};
