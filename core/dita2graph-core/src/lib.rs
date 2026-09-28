//! `dita2graph-core`: normalizes DITA-OT's resolved model (§3.2) into an
//! OKF v0.2 knowledge bundle (§4), per `docs/plugin-specification.md`.
//!
//! This crate implements Phase 1/2 of the roadmap in §12: the normalized
//! model contract, the OKF bundle writer, and the diagnostics catalog.
//! Relation *inference* (deriving edges DITA doesn't state explicitly,
//! §3.3) is fully implemented: `related-to` (shared `product` values)
//! and `applies-to` (matching `<uicontrol>` text between a task and a
//! reference topic, with an ambiguous match dropped rather than guessed)
//! both live in `relations.rs`. `generated-from` doesn't need inference
//! at all -- it's derived deterministically by the Java extractor from
//! DITA-OT's own `xtrf` source-trace attributes (finding 15). A SQLite
//! query-index (`store.rs`, opt-in via `build --store sqlite`) mirrors
//! `graph.json`'s nodes/edges for fast indexed lookups on a real corpus;
//! `graph.json` itself (a flattened, derived view, always written) stays
//! the default the `query` CLI subcommand reads. Incremental rebuild
//! (`incremental.rs`) skips rewriting an unchanged topic's concept file
//! and, when embeddings are configured, recomputing its embedding --
//! `graph.json`/`rag/chunks.jsonl`/`graph.db` are still always rewritten
//! in full every build, since their content must always reflect the
//! complete current node set regardless. RocksDB storage remains later
//! Phase 6+ work.

pub mod diagnostics;
pub mod embeddings;
pub(crate) mod incremental;
pub mod mcp_config;
pub mod model;
pub mod okf;
pub mod rag;
pub mod relations;
pub mod secrets;
pub mod store;

pub use embeddings::{
    Embedder, EmbeddingSummary, PreviousEmbeddings, cosine_similarity, write_embeddings_index,
};
pub use mcp_config::write_mcp_config;
pub use model::{Link, NormalizedMap, NormalizedNode, NormalizedTopic, Relation, TopicType};
pub use okf::{BundleSummary, write_bundle};
pub use rag::{RagSummary, write_rag_index};
pub use relations::{infer_applies_to, infer_related_to};
pub use secrets::{SecretFinding, scan_bundle};
pub use store::{StoreSummary, query_sqlite_store, read_sqlite_store, write_sqlite_store};
