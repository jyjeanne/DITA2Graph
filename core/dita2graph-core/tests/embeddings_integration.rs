//! Real end-to-end test against the toy ONNX fixture
//! (`tests/fixtures/embeddings/README.md`) — proves the pipeline
//! (tokenize -> ONNX inference -> mean-pool -> L2-normalize -> cosine
//! similarity) runs against a real ONNX Runtime, not just that the
//! surrounding Rust compiles. Skips (does not fail) when no usable
//! `libonnxruntime` is available, the same "skip with a message, don't
//! fail the suite" pattern `mcp/dita2graph-mcp/src/live.rs`'s vendored-
//! bundle tests already use for their own optional runtime dependency.

use dita2graph_core::{
    Embedder, NormalizedNode, NormalizedTopic, PreviousEmbeddings, TopicType, cosine_similarity,
    write_bundle, write_embeddings_index, write_rag_index,
};
use std::fs;
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

fn one_topic(id: &str, body: &str) -> Vec<NormalizedNode> {
    vec![NormalizedNode::Topic(NormalizedTopic {
        id: id.into(),
        topic_type: TopicType::Concept,
        title: id.into(),
        shortdesc: None,
        body: Some(body.into()),
        audience: vec![],
        product: vec![],
        keys: vec![],
        uicontrols: vec![],
        cmd_uicontrols: vec![],
        source_file: format!("topics/{id}.dita"),
        links: vec![],
    })]
}

/// Phase 6+'s "Incremental rebuild": a second `write_embeddings_index`
/// call for a topic whose chunk text hasn't changed must reuse the
/// stored vector instead of paying for another ONNX inference call --
/// proven against a real ONNX Runtime, not a mock, via the same
/// `embeddings_reused`/`embeddings_written` counters `dita2graph-core
/// build` reports on the CLI.
#[test]
fn write_embeddings_index_reuses_a_cached_vector_when_the_chunk_text_is_unchanged() {
    if !have_ort_dylib() {
        eprintln!("skipping: ORT_DYLIB_PATH not set (no ONNX Runtime available)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let embedder = Embedder::load(
        &fixture("tiny-embedding-model.onnx"),
        &fixture("tokenizer.json"),
    )
    .expect("loading toy embedding model + tokenizer");

    let nodes = one_topic("install-notes", "install product installing");
    write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
    write_rag_index(&nodes, dir.path(), chrono::Utc::now()).unwrap();

    // First build: nothing to reuse yet.
    let first = write_embeddings_index(
        &nodes,
        dir.path(),
        &embedder,
        "toy-fixture",
        &PreviousEmbeddings::default(),
    )
    .unwrap();
    assert_eq!(first.embeddings_written, 1);
    assert_eq!(first.embeddings_reused, 0);
    let first_vector = fs::read_to_string(dir.path().join("rag/embeddings.jsonl")).unwrap();

    // Second build, identical text: PreviousEmbeddings::load reads what
    // the first build just wrote (chunks.jsonl's text is unchanged,
    // write_rag_index wasn't re-run, so it's still the same file).
    let previous = PreviousEmbeddings::load(dir.path());
    let second =
        write_embeddings_index(&nodes, dir.path(), &embedder, "toy-fixture", &previous).unwrap();
    assert_eq!(second.embeddings_written, 0, "should reuse, not recompute");
    assert_eq!(second.embeddings_reused, 1);
    let second_vector = fs::read_to_string(dir.path().join("rag/embeddings.jsonl")).unwrap();
    assert_eq!(
        first_vector, second_vector,
        "the reused vector must be byte-identical to what was actually computed"
    );
}

/// The inverse: once a topic's body text actually changes, its stored
/// vector must not be reused -- a changed input demands a fresh
/// embedding, not a stale cached one silently reused because the id and
/// model name still match.
#[test]
fn write_embeddings_index_recomputes_once_the_chunk_text_changes() {
    if !have_ort_dylib() {
        eprintln!("skipping: ORT_DYLIB_PATH not set (no ONNX Runtime available)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let embedder = Embedder::load(
        &fixture("tiny-embedding-model.onnx"),
        &fixture("tokenizer.json"),
    )
    .expect("loading toy embedding model + tokenizer");

    let nodes = one_topic("install-notes", "install product installing");
    write_bundle(&nodes, dir.path(), chrono::Utc::now(), true).unwrap();
    write_rag_index(&nodes, dir.path(), chrono::Utc::now()).unwrap();
    write_embeddings_index(
        &nodes,
        dir.path(),
        &embedder,
        "toy-fixture",
        &PreviousEmbeddings::default(),
    )
    .unwrap();

    let previous = PreviousEmbeddings::load(dir.path());
    let changed_nodes = one_topic("install-notes", "weather forecast rain cloud sunny");
    // A real rebuild rewrites chunks.jsonl to match; skipped here since
    // this test only needs write_embeddings_index's own decision, which
    // reads `previous` (captured before this point), not chunks.jsonl
    // again.
    let summary = write_embeddings_index(
        &changed_nodes,
        dir.path(),
        &embedder,
        "toy-fixture",
        &previous,
    )
    .unwrap();
    assert_eq!(
        summary.embeddings_written, 1,
        "changed text must be re-embedded"
    );
    assert_eq!(summary.embeddings_reused, 0);
}
