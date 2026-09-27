//! Local ONNX sentence embeddings for `rag/` (§13.1's "node-level
//! embeddings" gap — the one piece of the hybrid graph+RAG architecture
//! §13.1 left as "design direction, not a committed design").
//!
//! Deliberately **not** the `download-binaries`/`fetch-models` path
//! `ort` offers: this crate builds and runs fully offline (`load-dynamic`
//! only pulls in `libloading`, no network at build or run time beyond
//! crates.io itself). The actual ONNX Runtime shared library and the
//! embedding model are both runtime inputs the operator supplies —
//! `ORT_DYLIB_PATH` (read by `ort` itself on first session creation) for
//! the former, `--embedding-model`/`--embedding-tokenizer` (`main.rs`)
//! for the latter — the same "bring your own" shape this project already
//! uses for DITA-OT itself and the vendored DitaCraft LSP bundle
//! (`mcp/dita2graph-mcp/vendor/ditacraft-lsp/README.md`), not a promise
//! that any particular model ships with this tool.
//!
//! No specific model is mandated. A real deployment would point this at
//! an ONNX export of a sentence-transformer such as `all-MiniLM-L6-v2`;
//! this module only assumes the export shape common to that family:
//! `input_ids`/`attention_mask` (`token_type_ids` too, if the model
//! declares it) in, and either pre-pooled sentence embeddings
//! (`[batch, dim]`) or raw per-token embeddings (`[batch, seq, dim]`,
//! mean-pooled here using `attention_mask` the same way
//! `sentence-transformers` itself does) out — detected from the output
//! tensor's rank rather than hardcoded to one export shape.

use crate::model::NormalizedNode;
use crate::rag::chunk_text;
use anyhow::{Context, Result, anyhow};
use ort::session::Session;
use ort::value::Tensor;
use serde::Serialize;
use std::cell::RefCell;
use std::fs;
use std::path::Path;

/// A loaded ONNX embedding model plus its matching tokenizer. `embed`
/// takes `&self` (not `&mut self`) via an interior `RefCell` around the
/// `ort::Session` — `ort::Session::run` needs `&mut Session`, but every
/// caller here (`build`'s per-chunk loop, `search_content`'s per-query
/// call) wants a shared, non-owning reference, matching the interior-
/// mutability pattern `BundleReader`'s own caches already use.
pub struct Embedder {
    session: RefCell<Session>,
    tokenizer: tokenizers::Tokenizer,
}

impl Embedder {
    /// Loads the ONNX model at `model_path` and the tokenizer at
    /// `tokenizer_path` (a HuggingFace `tokenizers` JSON file). Does not
    /// itself touch `ORT_DYLIB_PATH` or call `ort::init*` — `ort` reads
    /// `ORT_DYLIB_PATH` itself on first session creation when no
    /// environment has been explicitly committed, so the operator sets
    /// that env var and this constructor just works, with no
    /// environment-setup step of its own to get out of sync.
    pub fn load(model_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let session = Session::builder()
            .context("creating ONNX Runtime session builder")?
            .commit_from_file(model_path)
            .with_context(|| format!("loading ONNX model from {}", model_path.display()))?;
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow!("loading tokenizer from {}: {e}", tokenizer_path.display()))?;
        Ok(Embedder {
            session: RefCell::new(session),
            tokenizer,
        })
    }

    /// Embeds `text`, returning an L2-normalized vector (so
    /// [`cosine_similarity`] reduces to a plain dot product downstream).
    /// Errors on empty/all-whitespace text rather than silently returning
    /// a zero vector -- callers (`main.rs`'s per-chunk loop) skip chunks
    /// with no text before calling this, so an empty tokenization here
    /// means the model produced zero tokens from non-empty input, which
    /// is worth surfacing rather than masking.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Err(anyhow!("cannot embed empty/all-whitespace text"));
        }
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow!("tokenizing text: {e}"))?;
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        if ids.is_empty() {
            return Err(anyhow!("tokenizer produced zero tokens for non-empty text"));
        }
        let seq_len = ids.len();
        let mask: Vec<i64> = vec![1; seq_len];

        let mut session = self.session.borrow_mut();
        let wants_token_type_ids = session
            .inputs()
            .iter()
            .any(|input| input.name() == "token_type_ids");

        let input_ids = Tensor::from_array((vec![1i64, seq_len as i64], ids))
            .context("building input_ids tensor")?;
        let attention_mask = Tensor::from_array((vec![1i64, seq_len as i64], mask))
            .context("building attention_mask tensor")?;

        let mut inputs = vec![
            (
                "input_ids",
                ort::session::SessionInputValue::from(input_ids),
            ),
            (
                "attention_mask",
                ort::session::SessionInputValue::from(attention_mask),
            ),
        ];
        if wants_token_type_ids {
            let token_type_ids =
                Tensor::from_array((vec![1i64, seq_len as i64], vec![0i64; seq_len]))
                    .context("building token_type_ids tensor")?;
            inputs.push((
                "token_type_ids",
                ort::session::SessionInputValue::from(token_type_ids),
            ));
        }

        let output_name = session
            .outputs()
            .first()
            .map(|o| o.name().to_string())
            .ok_or_else(|| anyhow!("ONNX model declares no outputs"))?;

        let outputs = session
            .run(inputs)
            .context("running ONNX embedding inference")?;
        let (shape, data) = outputs[output_name.as_str()]
            .try_extract_tensor::<f32>()
            .context("extracting embedding output as f32 tensor")?;

        let pooled = match shape.len() {
            // Already pooled: [batch, dim].
            2 => data.to_vec(),
            // Raw per-token embeddings: [batch, seq, dim] -- mean-pool
            // over the sequence axis using the attention mask, the same
            // post-processing `sentence-transformers` itself applies to
            // this model family's raw export.
            3 => {
                let dim = shape[2] as usize;
                let actual_seq = shape[1] as usize;
                mean_pool(data, actual_seq, dim)
            }
            other => {
                return Err(anyhow!(
                    "unsupported embedding output rank {other} (expected 2 or 3 dims)"
                ));
            }
        };
        Ok(l2_normalize(&pooled))
    }
}

/// Mean-pools `data` (`[seq, dim]`, row-major, batch already stripped)
/// into one `[dim]` vector. Every position counts equally: the caller
/// never pads (one text in, one sequence out, no batching), so there is
/// no attention-mask-driven exclusion to apply here the way a padded
/// batch would need.
fn mean_pool(data: &[f32], seq: usize, dim: usize) -> Vec<f32> {
    let mut sum = vec![0f32; dim];
    for pos in 0..seq {
        for d in 0..dim {
            sum[d] += data[pos * dim + d];
        }
    }
    let count = seq.max(1) as f32;
    sum.iter().map(|v| v / count).collect()
}

fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

#[derive(Serialize)]
struct EmbeddingRecord<'a> {
    id: &'a str,
    model: &'a str,
    dim: usize,
    vector: Vec<f32>,
}

/// A written embeddings index's summary, for CLI reporting
/// (`dita2graph-core build`, mirroring [`crate::rag::RagSummary`]).
#[derive(Debug, Default)]
pub struct EmbeddingSummary {
    pub embeddings_written: usize,
    pub dim: usize,
}

/// Writes `<output_dir>/rag/embeddings.jsonl`: one record per topic that
/// has chunk text (the same `shortdesc`/`body` combination
/// `rag::write_rag_index` chunks, via the same [`chunk_text`] helper --
/// a topic with no text has nothing to embed and is skipped here --
/// unlike `chunks.jsonl`, which still writes a text-less record for
/// such a topic, so `embeddings.jsonl` is not guaranteed to have a
/// record for every id `chunks.jsonl` does; `search_content` already
/// treats a missing embeddings-map entry as "no semantic signal" for
/// that chunk, so this asymmetry is harmless). `model_name` is recorded
/// on every record so a later query-time embedder built from a
/// *different* model is at least
/// identifiable as such, even though the actual mismatch guard
/// ([`cosine_similarity`] returning `0.0` on a dimension mismatch) is
/// enforced structurally, not by checking this string.
pub fn write_embeddings_index(
    nodes: &[NormalizedNode],
    output_dir: &Path,
    embedder: &Embedder,
    model_name: &str,
) -> Result<EmbeddingSummary> {
    let rag_dir = output_dir.join("rag");
    fs::create_dir_all(&rag_dir).context("creating rag/")?;

    let mut lines = String::new();
    let mut summary = EmbeddingSummary::default();
    for node in nodes {
        let NormalizedNode::Topic(topic) = node else {
            continue;
        };
        let Some(text) = chunk_text(topic.shortdesc.as_deref(), topic.body.as_deref()) else {
            continue;
        };
        let vector = embedder
            .embed(&text)
            .with_context(|| format!("embedding topic `{}`", topic.id))?;
        summary.dim = vector.len();
        let record = EmbeddingRecord {
            id: &topic.id,
            model: model_name,
            dim: vector.len(),
            vector,
        };
        lines.push_str(&serde_json::to_string(&record).context("serializing embedding record")?);
        lines.push('\n');
        summary.embeddings_written += 1;
    }
    fs::write(rag_dir.join("embeddings.jsonl"), lines).context("writing rag/embeddings.jsonl")?;
    Ok(summary)
}

/// Cosine similarity between two vectors of equal length, in `[-1, 1]`.
/// Returns `0.0` for a length mismatch or a zero vector rather than
/// panicking or erroring -- callers (`search_content`'s per-chunk
/// scoring loop) treat a mismatched/degenerate embedding as "no semantic
/// signal", not a hard failure that would abort an otherwise-successful
/// keyword-only search.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_of_identical_vectors_is_one() {
        let v = vec![0.3, 0.4, 0.5];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_of_orthogonal_vectors_is_zero() {
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_of_opposite_vectors_is_negative_one() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_returns_zero_for_mismatched_lengths() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_similarity_returns_zero_for_zero_vector() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn mean_pool_averages_positions() {
        // seq=2, dim=2: [[1,3],[3,5]] -> mean [2,4]
        let data = [1.0, 3.0, 3.0, 5.0];
        assert_eq!(mean_pool(&data, 2, 2), vec![2.0, 4.0]);
    }

    #[test]
    fn l2_normalize_produces_unit_vector() {
        let v = l2_normalize(&[3.0, 4.0]);
        let norm = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_of_zero_vector_stays_zero() {
        assert_eq!(l2_normalize(&[0.0, 0.0]), vec![0.0, 0.0]);
    }
}
