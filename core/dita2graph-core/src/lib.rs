//! `dita2graph-core`: normalizes DITA-OT's resolved model (§3.2) into an
//! OKF v0.2 knowledge bundle (§4), per `docs/plugin-specification.md`.
//!
//! This crate implements Phase 1/2 of the roadmap in §12: the normalized
//! model contract, the OKF bundle writer, and the diagnostics catalog.
//! Relation *inference* (deriving edges DITA doesn't state explicitly,
//! §3.3) and the SQLite/RocksDB query index are later Phase 2 work and
//! are not yet implemented — `graph.json` (a flattened, derived view) is
//! written today and is enough for the `query` CLI subcommand.

pub mod diagnostics;
pub mod model;
pub mod okf;
pub mod secrets;

pub use model::{Link, NormalizedMap, NormalizedNode, NormalizedTopic, Relation, TopicType};
pub use okf::{BundleSummary, write_bundle};
pub use secrets::{SecretFinding, scan_bundle};
