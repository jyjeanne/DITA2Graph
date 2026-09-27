# Test fixture: toy embedding model

`tiny-embedding-model.onnx` + `tokenizer.json` are a **hand-built,
deterministic test fixture** for `src/embeddings.rs`'s integration test
(`tests/embeddings_integration.rs`), not a real sentence-transformer and
not suitable for production semantic search — its "vocabulary" is 16
hardcoded words and its "embeddings" are a fixed lookup table with no
learned language understanding. It exists only to prove the Rust
pipeline (tokenize → ONNX inference → mean-pool → L2-normalize → cosine
similarity) runs end to end against a real ONNX Runtime and produces the
ranking a working pipeline should, without committing a ~90 MB real
model to this repository.

Regenerate with `python3 build_model.py` (needs `pip install onnx
numpy`) if the fixture ever needs to change shape (e.g. to test a model
family with a different output rank).

## Vocabulary and clustering

16 tokens across 3 hand-assigned clusters (`build_model.py`'s `VOCAB` and
cluster-base vectors), each cluster's tokens embedded near a shared base
vector plus small random jitter — enough that a query built from
install-cluster tokens is reliably closer (cosine similarity) to a chunk
built from other install-cluster tokens than to one built from
weather-cluster tokens, which is what the integration test asserts.

## Using a real model instead

Point `dita2graph-core build --embedding-model <path.onnx>
--embedding-tokenizer <path/tokenizer.json>` (see `src/main.rs`) at a
real ONNX export instead — for example `sentence-transformers/all-
MiniLM-L6-v2` exported via `optimum-cli export onnx`. `Embedder::load`
(`src/embeddings.rs`) only assumes the `input_ids`/`attention_mask`(/
`token_type_ids`) in, pooled-or-raw-token-embeddings-out shape common to
that model family — it doesn't hardcode this fixture's vocabulary or
dimension.
