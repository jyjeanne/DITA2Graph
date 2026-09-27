//! Real end-to-end test against the toy ONNX fixture
//! (`tests/fixtures/embeddings/README.md`) — proves the pipeline
//! (tokenize -> ONNX inference -> mean-pool -> L2-normalize -> cosine
//! similarity) runs against a real ONNX Runtime, not just that the
//! surrounding Rust compiles. Skips (does not fail) when no usable
//! `libonnxruntime` is available, the same "skip with a message, don't
//! fail the suite" pattern `mcp/dita2graph-mcp/src/live.rs`'s vendored-
//! bundle tests already use for their own optional runtime dependency.

use dita2graph_core::{Embedder, cosine_similarity};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/embeddings"
    ))
    .join(name)
}

/// `ort` reads `ORT_DYLIB_PATH` itself on first session creation; this
/// just decides whether to attempt that at all, so a dev/CI environment
/// with no ONNX Runtime installed gets a skip instead of a hard failure.
fn have_ort_dylib() -> bool {
    std::env::var_os("ORT_DYLIB_PATH").is_some()
}

#[test]
fn embed_ranks_semantically_similar_text_closer_than_unrelated_text() {
    if !have_ort_dylib() {
        eprintln!("skipping: ORT_DYLIB_PATH not set (no ONNX Runtime available)");
        return;
    }

    let embedder = Embedder::load(
        &fixture("tiny-embedding-model.onnx"),
        &fixture("tokenizer.json"),
    )
    .expect("loading toy embedding model + tokenizer");

    let query = embedder.embed("install product").expect("embedding query");
    let install_chunk = embedder
        .embed("download the installer and run the install")
        .expect("embedding install-cluster chunk");
    let config_chunk = embedder
        .embed("configuration settings overview")
        .expect("embedding config-cluster chunk");
    let weather_chunk = embedder
        .embed("weather forecast rain cloud sunny temperature")
        .expect("embedding weather-cluster chunk");

    let sim_install = cosine_similarity(&query, &install_chunk);
    let sim_config = cosine_similarity(&query, &config_chunk);
    let sim_weather = cosine_similarity(&query, &weather_chunk);

    assert!(
        sim_install > sim_weather,
        "install-cluster chunk ({sim_install}) should rank above weather-cluster chunk ({sim_weather}) for an install-cluster query"
    );
    assert!(
        sim_install > sim_config,
        "install-cluster chunk ({sim_install}) should rank above config-cluster chunk ({sim_config}) for an install-cluster query"
    );
    // Every fixture embedding is L2-normalized, so a self-comparison
    // (same text through the same pipeline twice) must reproduce
    // exactly -- proves determinism, not just "some similarity ordering
    // happened to come out right".
    let install_chunk_again = embedder
        .embed("download the installer and run the install")
        .expect("re-embedding the same text");
    assert!(
        (cosine_similarity(&install_chunk, &install_chunk_again) - 1.0).abs() < 1e-5,
        "embedding the same text twice should be deterministic"
    );
}

#[test]
fn embed_errors_on_empty_text() {
    if !have_ort_dylib() {
        eprintln!("skipping: ORT_DYLIB_PATH not set (no ONNX Runtime available)");
        return;
    }

    let embedder = Embedder::load(
        &fixture("tiny-embedding-model.onnx"),
        &fixture("tokenizer.json"),
    )
    .expect("loading toy embedding model + tokenizer");

    assert!(embedder.embed("").is_err());
}
