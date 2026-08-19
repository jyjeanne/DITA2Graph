//! A minimal LSP *client* that spawns DitaCraft's standalone LSP server
//! bundle (vendored at `vendor/ditacraft-lsp/` -- see that directory's
//! `README.md` for provenance and why it's vendored rather than
//! adapted, unlike `main.rs`'s `okf-mcp`-derived transport) and pulls
//! live diagnostics for one DITA source file. Backs the `validate_live`
//! tool (`tools.rs`): the same 13-phase validation pipeline DitaCraft
//! runs as-you-type in VS Code (DTD/RNG, 43 Schematron-equivalent
//! rules, cross-reference, circular-reference, subject-scheme
//! profiling), now callable from an AI agent against a topic's
//! *current* on-disk source -- complementing `validate_bundle`, which
//! only re-checks the *last build's* OKF/secret-leak gates.
//!
//! Unlike MCP's own stdio framing (one JSON-RPC message per line, see
//! `main.rs`), LSP uses HTTP-style `Content-Length` headers (LSP 3.17).
//! There's no existing framing helper for that anywhere in this
//! workspace, and pulling in a full LSP/async-runtime dependency for
//! one request/response exchange per tool call isn't worth it, so this
//! implements it directly against plain `Read`/`Write`/`BufRead`.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a single `validate_file` call is allowed to take before its
/// child `node` process is killed and the call fails with a timeout
/// error. The pipeline itself typically finishes in single-digit
/// milliseconds (`docs/DITA_LSP_ARCHITECTURE.md`'s own
/// `[validation] ... Total=9ms` example) -- this bounds Node startup
/// plus a pathological document, not normal operation.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

/// How long to wait after `textDocument/didOpen` before the *first*
/// diagnostic pull -- past DitaCraft's own 300ms validation debounce,
/// with margin for its async key-space BFS across a map hierarchy. See
/// the call site in `run_session` for what was actually observed
/// without this delay. Not the only wait: `run_session` keeps polling
/// past this point (`DIAGNOSTIC_POLL_INTERVAL`/`MAX_DIAGNOSTIC_POLLS`
/// below) until results stop changing, since a single fixed delay isn't
/// long enough for every map size.
const DIAGNOSTIC_SETTLE_DELAY: Duration = Duration::from_millis(600);

/// Interval between re-pulls once the settle delay has elapsed.
const DIAGNOSTIC_POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Upper bound on re-pulls after the first one -- caps the extra time
/// this can add at `MAX_DIAGNOSTIC_POLLS * DIAGNOSTIC_POLL_INTERVAL`
/// (2.4s), comfortably inside `RESPONSE_TIMEOUT`. A document whose
/// diagnostics are still changing after this many polls just returns
/// its latest snapshot rather than polling forever.
const MAX_DIAGNOSTIC_POLLS: u32 = 6;

/// Where to find the vendored DitaCraft LSP bundle and how to invoke
/// it. `Default` points at this crate's own `vendor/ditacraft-lsp/`
/// (correct for `cargo run`/`cargo test` from a source checkout);
/// `--ditacraft-lsp-root`/`DITA2GRAPH_DITACRAFT_LSP_ROOT` override it
/// for an installed binary that doesn't ship next to its source tree.
#[derive(Clone, Debug)]
pub struct LiveValidationConfig {
    /// Root of the original DITA source project that `resource` paths
    /// in OKF frontmatter (§4.4, `okf.rs`'s `Frontmatter.resource`) are
    /// relative to. `None` until `--source-root`/`DITA2GRAPH_SOURCE_ROOT`
    /// (or a `[dita]` config section) is given -- `validate_live`
    /// reports a clear configuration error rather than guessing at a
    /// project root.
    pub source_root: Option<PathBuf>,
    /// Directory containing `dist/lsp-server.js` and `dtds/` (the
    /// layout `vendor/ditacraft-lsp/README.md` and upstream
    /// `server/src/standalone.ts` both document).
    pub lsp_root: PathBuf,
    /// The `node` executable to spawn (PATH-resolved by default).
    pub node_bin: String,
}

impl Default for LiveValidationConfig {
    fn default() -> Self {
        LiveValidationConfig {
            source_root: None,
            lsp_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/ditacraft-lsp")),
            node_bin: "node".to_string(),
        }
    }
}

/// One LSP `Diagnostic` (LSP 3.17) -- only the fields `validate_live`'s
/// formatting actually uses.
#[derive(Deserialize, Debug)]
pub struct LiveDiagnostic {
    pub range: Range,
    pub message: String,
    #[serde(default)]
    pub severity: Option<u8>,
    #[serde(default)]
    pub code: Option<Value>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Range {
    pub start: Position,
}

#[derive(Deserialize, Debug)]
pub struct Position {
    pub line: u64,
}

/// LSP `DiagnosticSeverity` (1=Error .. 4=Hint) as the lowercase label
/// DitaCraft's own Problems-panel/status-bar language uses.
pub fn severity_label(severity: Option<u8>) -> &'static str {
    match severity {
        Some(1) => "error",
        Some(2) => "warning",
        Some(3) => "info",
        Some(4) => "hint",
        _ => "unknown",
    }
}

/// Renders a diagnostic `code` (usually a bare JSON string like
/// `"DITA-STRUCT-001"`, but the LSP spec also allows a number or a
/// `{value, target}` object) as plain text for tool output.
pub fn code_str(code: &Value) -> String {
    match code {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| code.to_string()),
        other => other.to_string(),
    }
}

/// Spawns the vendored LSP server, opens `file_path` (must exist on
/// disk, and should live under `workspace_root` so DitaCraft's own
/// key-space/cross-reference resolution can find sibling maps) inside
/// `workspace_root`, pulls diagnostics for it (LSP 3.17 pull
/// diagnostics, `textDocument/diagnostic` -- the same request
/// DitaCraft's own client issues, per `docs/DITA_LSP_ARCHITECTURE.md`'s
/// "Diagnostics are pull-based" note), then shuts the server down.
///
/// One process per call, deliberately: `dita2graph-mcp` handles one
/// JSON-RPC request at a time (`main.rs`'s stdin loop), so there is no
/// concurrent-call scenario a pooled/warm process would help with yet,
/// and spawn-per-call keeps this module free of the lifecycle
/// bookkeeping a shared, long-lived server process would need (crash
/// recovery, workspace-change notifications, ...). Worth revisiting if
/// per-call Node startup cost becomes a real bottleneck.
pub fn validate_file(
    config: &LiveValidationConfig,
    workspace_root: &Path,
    file_path: &Path,
) -> Result<Vec<LiveDiagnostic>> {
    let bundle_path = config.lsp_root.join("dist").join("lsp-server.js");
    if !bundle_path.is_file() {
        return Err(anyhow!(
            "DitaCraft LSP bundle not found at {} -- set --ditacraft-lsp-root or \
             DITA2GRAPH_DITACRAFT_LSP_ROOT, or vendor it at vendor/ditacraft-lsp/ \
             (see vendor/ditacraft-lsp/README.md)",
            bundle_path.display()
        ));
    }
    let text = std::fs::read_to_string(file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;

    // `file://` URIs need absolute paths -- a relative `workspace_root`/
    // `file_path` (e.g. `--source-root` given as a relative path on the
    // command line) would otherwise produce a malformed URI whose first
    // path segment gets parsed as the URI's *host* instead of part of
    // the path, silently breaking every href/keyref resolution inside
    // the LSP (confirmed live: every cross-reference in a real topic
    // came back "target not found" until this was added, even though
    // the targets existed exactly where the map said).
    let workspace_root = workspace_root
        .canonicalize()
        .with_context(|| format!("resolving {} to an absolute path", workspace_root.display()))?;
    let file_path = file_path
        .canonicalize()
        .with_context(|| format!("resolving {} to an absolute path", file_path.display()))?;

    let mut child = Command::new(&config.node_bin)
        .arg(&bundle_path)
        .arg("--stdio")
        .current_dir(&workspace_root)
        .env("DITACRAFT_EXTENSION_ROOT", &config.lsp_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "spawning `{} {} --stdio` -- is Node.js installed and on PATH?",
                config.node_bin,
                bundle_path.display()
            )
        })?;

    // Drain stderr continuously on its own thread for the whole session,
    // not just at the end: `Stdio::piped()` gives it a fixed-size OS
    // pipe buffer (~64KB on Linux), and nothing else in this module ever
    // reads it. Left undrained, a verbose child (console.error/warn
    // calls exist in the vendored bundle, or a Node stack trace) fills
    // that buffer and blocks on its next stderr write -- while this
    // process sits blocked reading stdout for a response that child can
    // now never produce, silently eating the full RESPONSE_TIMEOUT and
    // reporting a misleading "timed out" error instead of the real
    // stderr content. Captured (not discarded) so a genuine failure's
    // error message can include it.
    let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_thread = child.stderr.take().map(|mut stderr| {
        let stderr_buf = Arc::clone(&stderr_buf);
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            if let Ok(mut captured) = stderr_buf.lock() {
                *captured = buf;
            }
        })
    });

    let result = run_session_with_timeout(&mut child, &workspace_root, &file_path, &text);

    // Best-effort cleanup regardless of how the session ended -- an
    // error or timeout mid-handshake must not leak a live node process.
    let _ = child.kill();
    let _ = child.wait();
    // The stderr pipe closes once the (now-killed) child has fully
    // exited, so the reader thread above is done shortly after `wait()`
    // returns -- join it so the captured buffer below is complete.
    if let Some(stderr_thread) = stderr_thread {
        let _ = stderr_thread.join();
    }

    match result {
        Ok(diagnostics) => Ok(diagnostics),
        Err(e) => {
            let captured = stderr_buf
                .lock()
                .map(|buf| String::from_utf8_lossy(&buf).trim().to_string())
                .unwrap_or_default();
            if captured.is_empty() {
                Err(e)
            } else {
                Err(e.context(format!("ditacraft-lsp stderr:\n{captured}")))
            }
        }
    }
}

/// Runs the handshake on a worker thread and bounds the whole exchange
/// by `RESPONSE_TIMEOUT` -- `read_message` below blocks on pipe reads
/// with no built-in deadline (pipes, unlike sockets, don't expose a
/// portable read-timeout in `std`), so a wedged child process is
/// contained by racing its result against `recv_timeout` instead of by
/// interrupting the read itself.
fn run_session_with_timeout(
    child: &mut Child,
    workspace_root: &Path,
    file_path: &Path,
    text: &str,
) -> Result<Vec<LiveDiagnostic>> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no stdin on ditacraft-lsp child process"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("no stdout on ditacraft-lsp child process"))?;

    let workspace_root = workspace_root.to_path_buf();
    let file_path = file_path.to_path_buf();
    let text = text.to_string();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = run_session(stdin, stdout, &workspace_root, &file_path, &text);
        let _ = tx.send(result);
    });

    rx.recv_timeout(RESPONSE_TIMEOUT)
        .unwrap_or_else(|_| Err(anyhow!("timed out waiting for ditacraft-lsp to respond")))
}

fn run_session(
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    workspace_root: &Path,
    file_path: &Path,
    text: &str,
) -> Result<Vec<LiveDiagnostic>> {
    let mut reader = std::io::BufReader::new(stdout);

    let root_uri = path_to_file_uri(workspace_root)?;
    let file_uri = path_to_file_uri(file_path)?;

    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {},
                "workspaceFolders": [{ "uri": root_uri, "name": "workspace" }],
            },
        }),
    )?;
    read_response(&mut reader, 1)?;

    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )?;

    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": file_uri, "languageId": "dita", "version": 1, "text": text,
                },
            },
        }),
    )?;

    // DitaCraft's own validation pipeline is debounced (300ms for
    // topics, `docs/DITA_LSP_ARCHITECTURE.md`'s "smart debounce
    // mechanism"), and cross-reference/keyref checks additionally
    // depend on its async KeySpaceService having finished a BFS map
    // traversal it only starts once a document opens. Pulling
    // `textDocument/diagnostic` immediately after `didOpen` -- before
    // either has settled -- was confirmed live to return spurious
    // DITA-XREF-001/DITA-KEY-001 findings for targets and keys that do
    // exist, on a topic whose map hasn't finished resolving yet.
    // There's no server-pushed "diagnostics are ready" signal
    // implemented in the vendored bundle to wait on instead
    // (`workspace/diagnostic/refresh` is sent immediately on open,
    // before validation finishes, not after -- confirmed by log message
    // ordering). So: wait past the debounce window, then keep re-pulling
    // until two consecutive pulls agree (or the poll cap is hit) --
    // a single fixed delay only covers documents whose key-space BFS
    // finishes within it; a larger map hierarchy can still be resolving
    // past that point.
    std::thread::sleep(DIAGNOSTIC_SETTLE_DELAY);

    let mut next_id: u64 = 2;
    let mut items = pull_diagnostics(&mut stdin, &mut reader, &file_uri, next_id)?;
    for _ in 0..MAX_DIAGNOSTIC_POLLS {
        std::thread::sleep(DIAGNOSTIC_POLL_INTERVAL);
        next_id += 1;
        let next_items = pull_diagnostics(&mut stdin, &mut reader, &file_uri, next_id)?;
        if next_items == items {
            break;
        }
        items = next_items;
    }

    // Best-effort from here on: the diagnostics we actually came for are
    // already in `items` above. `shutdown`/`exit` are pure cleanup
    // courtesy to the child (LSP spec) -- a broken pipe on either write
    // (e.g. the child exiting on its own right after answering the last
    // diagnostic request) must not turn a *successful* call into an
    // error and discard the diagnostics we already have.
    // `validate_file`'s `child.kill()`/`wait()` reap the process
    // regardless of whether this handshake completes cleanly.
    next_id += 1;
    let _ = write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": next_id, "method": "shutdown", "params": null }),
    );
    let _ = read_response(&mut reader, next_id);
    let _ = write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "exit", "params": null }),
    );

    serde_json::from_value(items).context("parsing textDocument/diagnostic result items")
}

/// Sends one `textDocument/diagnostic` pull request with the given
/// request `id` and returns its `items` array (or an empty one if the
/// response omitted it).
fn pull_diagnostics<R: BufRead>(
    stdin: &mut ChildStdin,
    reader: &mut R,
    file_uri: &str,
    id: u64,
) -> Result<Value> {
    write_message(
        stdin,
        &json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/diagnostic",
            "params": { "textDocument": { "uri": file_uri } },
        }),
    )?;
    let result = read_response(reader, id)?;
    Ok(result.get("items").cloned().unwrap_or_else(|| json!([])))
}

/// Converts an absolute filesystem path to a `file://` URI,
/// percent-encoding every byte outside RFC 3986's unreserved set
/// (letters, digits, `-._~`), `/` aside (kept as the path separator).
/// A raw `format!("file://{}", path.display())` left a space, `#`, `%`,
/// `?`, or any non-ASCII byte unencoded -- the vendored LSP's own URI
/// handling (`vscode-uri`, used to compare href/keyref targets against
/// document URIs) treats those specially (`#` starts a fragment; a
/// literal space or `%` is simply invalid unescaped), so a workspace or
/// topic file whose path contains one broke every cross-reference
/// lookup against it, the same "target not found" false positive the
/// `canonicalize()` call above exists to prevent for relative paths.
fn path_to_file_uri(path: &Path) -> Result<String> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("{} is not valid UTF-8", path.display()))?;
    let mut uri = String::from("file://");
    for byte in path_str.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char);
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    Ok(uri)
}

fn write_message<W: Write>(w: &mut W, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

/// Reads framed messages until one whose `id` equals `expected_id`
/// arrives, discarding server-to-client notifications
/// (`window/logMessage`, `workspace/diagnostic/refresh`, etc. -- the
/// DitaCraft LSP sends several of these on startup) along the way.
fn read_response<R: BufRead>(reader: &mut R, expected_id: u64) -> Result<Value> {
    loop {
        let message = read_message(reader)?;
        if message.get("id").and_then(Value::as_u64) == Some(expected_id) {
            if let Some(error) = message.get("error") {
                return Err(anyhow!("ditacraft-lsp returned an error: {error}"));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
        // A notification, or a response to some other id -- keep reading.
    }
}

/// Parses one `Content-Length: N\r\n\r\n<N bytes of JSON>` frame.
fn read_message<R: BufRead>(reader: &mut R) -> Result<Value> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .context("reading an LSP header line from ditacraft-lsp")?;
        if n == 0 {
            return Err(anyhow!(
                "ditacraft-lsp closed its stdout before sending a complete message"
            ));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse()
                    .context("parsing Content-Length header")?,
            );
        }
        // Other headers (e.g. Content-Type) are valid per LSP but unused here.
    }
    let len = content_length
        .ok_or_else(|| anyhow!("ditacraft-lsp message had no Content-Length header"))?;
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .context("reading an LSP message body from ditacraft-lsp")?;
    serde_json::from_slice(&body).context("parsing an LSP message body as JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn write_then_read_round_trips_a_message() {
        let mut buf: Vec<u8> = Vec::new();
        let value = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        write_message(&mut buf, &value).unwrap();

        assert!(buf.starts_with(b"Content-Length: "));
        let mut reader = Cursor::new(buf);
        let parsed = read_message(&mut reader).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn read_message_rejects_a_missing_content_length_header() {
        let mut reader = Cursor::new(b"Content-Type: application/json\r\n\r\n{}".to_vec());
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn read_message_errors_on_truncated_stream() {
        // Declares 100 bytes but the stream has none -- must error, not hang.
        let mut reader = Cursor::new(b"Content-Length: 100\r\n\r\n".to_vec());
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn read_response_skips_notifications_before_the_matching_id() {
        let mut buf: Vec<u8> = Vec::new();
        write_message(
            &mut buf,
            &json!({ "jsonrpc": "2.0", "method": "window/logMessage", "params": { "message": "hi" } }),
        )
        .unwrap();
        write_message(
            &mut buf,
            &json!({ "jsonrpc": "2.0", "id": 2, "result": { "items": [] } }),
        )
        .unwrap();

        let mut reader = Cursor::new(buf);
        let result = read_response(&mut reader, 2).unwrap();
        assert_eq!(result, json!({ "items": [] }));
    }

    #[test]
    fn read_response_surfaces_a_json_rpc_error() {
        let mut buf: Vec<u8> = Vec::new();
        write_message(
            &mut buf,
            &json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32000, "message": "boom" } }),
        )
        .unwrap();

        let mut reader = Cursor::new(buf);
        let err = read_response(&mut reader, 1).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn code_str_unwraps_a_bare_string() {
        assert_eq!(code_str(&json!("DITA-STRUCT-001")), "DITA-STRUCT-001");
    }

    #[test]
    fn code_str_unwraps_a_value_target_object() {
        assert_eq!(
            code_str(&json!({ "value": "DITA-STRUCT-001", "target": "https://example.com" })),
            "DITA-STRUCT-001"
        );
    }

    #[test]
    fn severity_label_covers_all_four_lsp_severities() {
        assert_eq!(severity_label(Some(1)), "error");
        assert_eq!(severity_label(Some(2)), "warning");
        assert_eq!(severity_label(Some(3)), "info");
        assert_eq!(severity_label(Some(4)), "hint");
        assert_eq!(severity_label(None), "unknown");
    }

    #[test]
    fn path_to_file_uri_leaves_unreserved_characters_unencoded() {
        let uri = path_to_file_uri(Path::new("/home/user/project/topic.dita")).unwrap();
        assert_eq!(uri, "file:///home/user/project/topic.dita");
    }

    #[test]
    fn path_to_file_uri_percent_encodes_a_space() {
        // Regression test: a raw `file://{path}` left spaces unencoded,
        // which the vendored LSP's URI handling doesn't accept as part
        // of a path -- breaking every href/keyref comparison against a
        // workspace or topic path containing one (e.g. "My Docs").
        let uri = path_to_file_uri(Path::new("/home/user/My Docs/topic.dita")).unwrap();
        assert_eq!(uri, "file:///home/user/My%20Docs/topic.dita");
    }

    #[test]
    fn path_to_file_uri_percent_encodes_reserved_uri_characters() {
        // '#' starts a URI fragment, '%' begins a percent-escape, and
        // '?' starts a query string -- all three must themselves be
        // encoded when they're literal path characters, or the
        // resulting URI's path is truncated/misparsed at that point.
        let uri = path_to_file_uri(Path::new("/docs/install guide #2 (100%)?.dita")).unwrap();
        assert_eq!(
            uri,
            "file:///docs/install%20guide%20%232%20%28100%25%29%3F.dita"
        );
    }

    #[test]
    fn path_to_file_uri_percent_encodes_non_ascii_bytes() {
        let uri = path_to_file_uri(Path::new("/docs/café.dita")).unwrap();
        // "é" is the two UTF-8 bytes 0xC3 0xA9.
        assert_eq!(uri, "file:///docs/caf%C3%A9.dita");
    }

    /// Real end-to-end smoke test against the vendored bundle. Skips
    /// (rather than fails) when `node` isn't on `PATH` -- `rust.yml`
    /// doesn't install Node.js (only `integration.yml`'s DITA-OT job
    /// needs a non-Rust toolchain), so this must degrade gracefully
    /// there instead of breaking the default CI job.
    #[test]
    fn validate_file_runs_the_real_vendored_bundle_end_to_end() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping: no `node` on PATH");
            return;
        }
        let config = LiveValidationConfig::default();
        if !config.lsp_root.join("dist").join("lsp-server.js").is_file() {
            eprintln!(
                "skipping: vendored bundle not found at {}",
                config.lsp_root.display()
            );
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let topic_path = dir.path().join("sample.dita");
        std::fs::write(
            &topic_path,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <concept><title>Sample</title><conbody><p>hi</p></conbody></concept>\n",
        )
        .unwrap();

        let diagnostics = validate_file(&config, dir.path(), &topic_path).unwrap();

        // No DOCTYPE and no id on the root -- the same two structural
        // findings the manual smoke test against this bundle produced.
        assert!(
            diagnostics.iter().any(|d| d
                .code
                .as_ref()
                .is_some_and(|c| code_str(c) == "DITA-STRUCT-001")),
            "expected a missing-DOCTYPE diagnostic, got: {diagnostics:#?}"
        );
        assert!(
            diagnostics.iter().any(|d| d
                .code
                .as_ref()
                .is_some_and(|c| code_str(c) == "DITA-STRUCT-003")),
            "expected a missing-id diagnostic, got: {diagnostics:#?}"
        );
    }

    /// Regression test for a real bug found by manually running
    /// `validate_live` against this repo's own `sample-docs/`: a
    /// *relative* `workspace_root`/`file_path` produced a malformed
    /// `file://` URI (its first path segment parsed as the URI's host,
    /// not part of the path), which silently broke every href/keyref
    /// lookup inside the LSP -- `xref href="configuration.dita"` came
    /// back "target not found" even though `topics/configuration.dita`
    /// existed exactly where `sample-docs/user-guide.ditamap` said.
    /// `validate_file` now canonicalizes both paths before building
    /// URIs; this exercises that fix with a *deliberately relative*
    /// `workspace_root`/`file_path` pair, a real map, and real
    /// cross-file `xref`/`keyref` targets -- the single-file,
    /// pre-absolute-tempdir case above wouldn't have caught this.
    #[test]
    fn validate_file_resolves_cross_references_given_a_relative_workspace_root() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping: no `node` on PATH");
            return;
        }
        let config = LiveValidationConfig::default();
        if !config.lsp_root.join("dist").join("lsp-server.js").is_file() {
            eprintln!(
                "skipping: vendored bundle not found at {}",
                config.lsp_root.display()
            );
            return;
        }

        // CARGO_MANIFEST_DIR is mcp/dita2graph-mcp/ -- sample-docs/ is
        // two levels up, at the repo root. Relative, on purpose: this
        // is exactly the form `--source-root sample-docs` takes on the
        // command line from the repo root.
        let workspace_root = PathBuf::from("../../sample-docs");
        let file_path = workspace_root.join("topics/installing-product.dita");
        assert!(
            file_path.is_file(),
            "fixture moved? expected {} to exist",
            file_path.display()
        );

        let diagnostics = validate_file(&config, &workspace_root, &file_path).unwrap();

        assert!(
            diagnostics.iter().all(|d| d.code.as_ref().is_none_or(|c| {
                let c = code_str(c);
                c != "DITA-XREF-001" && c != "DITA-KEY-001"
            })),
            "cross-reference/key lookups should have resolved against the real map, \
             got: {diagnostics:#?}"
        );
    }

    /// Regression test for the `path_to_file_uri` unit tests above,
    /// exercised through the real vendored bundle: a workspace root
    /// containing a space (a common, entirely valid path on every
    /// platform this targets) must not turn into `file://` URIs the
    /// LSP's own URI handling mis-resolves. Same fixture as the
    /// relative-path test above, copied into a directory whose name has
    /// a space in it.
    #[test]
    fn validate_file_resolves_cross_references_given_a_workspace_root_with_a_space() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping: no `node` on PATH");
            return;
        }
        let config = LiveValidationConfig::default();
        if !config.lsp_root.join("dist").join("lsp-server.js").is_file() {
            eprintln!(
                "skipping: vendored bundle not found at {}",
                config.lsp_root.display()
            );
            return;
        }

        let parent = tempfile::tempdir().unwrap();
        let workspace_root = parent.path().join("sample docs");
        copy_dir_recursive(Path::new("../../sample-docs"), &workspace_root).unwrap();
        let file_path = workspace_root.join("topics/installing-product.dita");
        assert!(file_path.is_file());

        let diagnostics = validate_file(&config, &workspace_root, &file_path).unwrap();

        assert!(
            diagnostics.iter().all(|d| d.code.as_ref().is_none_or(|c| {
                let c = code_str(c);
                c != "DITA-XREF-001" && c != "DITA-KEY-001"
            })),
            "cross-reference/key lookups should have resolved even with a space \
             in the workspace path, got: {diagnostics:#?}"
        );
    }

    fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let dest = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir_recursive(&entry.path(), &dest)?;
            } else {
                std::fs::copy(entry.path(), dest)?;
            }
        }
        Ok(())
    }

    /// Regression test for a since-fixed upstream bug, reported as
    /// `jyjeanne/ditacraft#125` and previously documented in
    /// `vendor/ditacraft-lsp/KNOWN-ISSUES.md` -- found by running
    /// `validate_live` against every real topic in the DITA-OT project's
    /// own documentation (`dita-ot/docs`, 267 topics; this was the one
    /// exception out of all of them). The vendored LSP's own process
    /// (not `dita2graph-mcp`) used to peg a full CPU core and never
    /// respond for this exact `<codeblock>` content (real content,
    /// bisected down from the real file that triggered it), reliably
    /// timing out at the full `RESPONSE_TIMEOUT` (20s). Fixed upstream
    /// in `jyjeanne/ditacraft` v0.9.1 -- confirmed by re-vendoring that
    /// release's `dist/lsp-server.js` and re-running this exact test,
    /// which now completes in ~1.3s instead of timing out at 20s. Kept
    /// as a permanent regression test (not deleted) so a future vendored
    /// bundle update that reintroduces this can't land silently.
    #[test]
    fn validate_file_no_longer_hangs_on_a_real_codeblock_from_dita_ot_docs() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping: no `node` on PATH");
            return;
        }
        let config = LiveValidationConfig::default();
        if !config.lsp_root.join("dist").join("lsp-server.js").is_file() {
            eprintln!(
                "skipping: vendored bundle not found at {}",
                config.lsp_root.display()
            );
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let topic_path = dir.path().join("plugin-entry.dita");
        std::fs::write(
            &topic_path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE topic PUBLIC "-//OASIS//DTD DITA Topic//EN" "topic.dtd">
<topic id="plugin-entry">
  <title>Sample plug-in entry file</title>
  <body>
    <codeblock outputclass="language-json" xml:space="preserve">[
  {
    "name": "org.dita.docbook",
    "description": "Convert DITA to DocBook.",
    "keywords": ["DocBook"],
    "homepage": "https://github.com/dita-ot/org.dita.docbook/",
    "vers": "2.3.0",
    "license": "Apache-2.0",
    "deps": [
      {
        "name": "org.dita.base",
        "req": ">=2.3.0"
      }
    ],
    "url": "https://github.com/dita-ot/org.dita.docbook/archive/2.3.zip",
    "cksum": "eaf06b0dca8d942bd4152615e39ee8cfb73a624b96d70e10ab269ed6f8a13e21"
  }
]</codeblock>
  </body>
</topic>
"#,
        )
        .unwrap();

        let started = std::time::Instant::now();
        let diagnostics = validate_file(&config, dir.path(), &topic_path).expect(
            "validate_file should complete normally now -- if this times out again, the \
             CPU-spin bug (jyjeanne/ditacraft#125) has regressed in whatever bundle is \
             currently vendored",
        );
        let elapsed = started.elapsed();

        // A well-formed, DOCTYPE-declared, id-bearing topic with a
        // single codeblock: genuinely no diagnostics expected.
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics for this well-formed fixture, got: {diagnostics:#?}"
        );
        // The bug's signature was pegging a CPU core for the entire
        // RESPONSE_TIMEOUT (20s); completing in a small fraction of
        // that is the actual regression signal, not just "didn't hit
        // the timeout error path" (which a much slower but still-under-
        // timeout fix could also satisfy without truly being fixed).
        assert!(
            elapsed < Duration::from_secs(5),
            "expected this to complete quickly (previously it pegged a CPU core for the \
             full 20s timeout); took {elapsed:?}"
        );
    }
}
