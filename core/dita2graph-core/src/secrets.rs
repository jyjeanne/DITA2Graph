//! Secret-leakage detection (docs/plugin-specification.md §6.4): scans a
//! generated OKF bundle for common, high-confidence secret patterns
//! before it's considered valid. `okf-validator` (an external dependency,
//! §3) has no equivalent rule and isn't ours to extend, so this lives
//! here, run alongside it (§2.5, `validate`/`build` in `main.rs`).
//!
//! Deliberately narrow: only prefixes specific and structured enough that
//! a false positive is very unlikely (AWS access key IDs, PEM private-key
//! headers, GitHub and Slack tokens). No generic `password=`/`api_key=`
//! heuristic — that would false-positive on ordinary documentation prose
//! (a topic titled "How to reset your password" is not a leak), which is
//! exactly the kind of guessy pattern-matching this codebase has avoided
//! elsewhere (see `DitaModelExtractor`'s relation-extraction scope, Java
//! side). A secret already redacted or partially obscured in source
//! prose (e.g. "your API key, e.g. AKIA...") will still match; that's a
//! false positive this scanner accepts in exchange for never missing a
//! real one shaped like the patterns below.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct SecretFinding {
    pub file: String,
    pub pattern: &'static str,
}

const PRIVATE_KEY_HEADERS: &[&str] = &[
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "-----BEGIN DSA PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN PGP PRIVATE KEY BLOCK-----",
];

const GITHUB_TOKEN_PREFIXES: &[&str] = &["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"];

const SLACK_TOKEN_PREFIXES: &[&str] = &["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"];

/// Scans every file in `bundle_dir` (recursively) for the patterns above.
/// Returns one finding per (file, pattern) match — a file can appear more
/// than once if it trips more than one pattern.
pub fn scan_bundle(bundle_dir: &Path) -> Result<Vec<SecretFinding>> {
    let mut findings = Vec::new();
    let mut files = Vec::new();
    collect_files(bundle_dir, &mut files)?;
    files.sort();

    for path in files {
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue, // binary/non-UTF-8 file; nothing to scan as text
        };
        let relative = path
            .strip_prefix(bundle_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if let Some(pattern) = scan_text(&content) {
            findings.push(SecretFinding {
                file: relative,
                pattern,
            });
        }
    }
    Ok(findings)
}

fn scan_text(content: &str) -> Option<&'static str> {
    if find_aws_access_key(content) {
        return Some("AWS access key ID (AKIA...)");
    }
    for header in PRIVATE_KEY_HEADERS {
        if content.contains(header) {
            return Some("PEM private key block");
        }
    }
    for prefix in GITHUB_TOKEN_PREFIXES {
        if find_token(content, prefix, 36) {
            return Some("GitHub access token");
        }
    }
    for prefix in SLACK_TOKEN_PREFIXES {
        if find_token(content, prefix, 10) {
            return Some("Slack token");
        }
    }
    None
}

/// `AKIA` followed by 16 uppercase-alphanumeric characters, word-bounded
/// (not itself a substring of a longer alphanumeric run) -- the fixed
/// shape of a real AWS access key ID.
fn find_aws_access_key(content: &str) -> bool {
    const PREFIX: &str = "AKIA";
    const KEY_LEN: usize = 20; // "AKIA" + 16 chars
    let bytes = content.as_bytes();
    let mut start = 0;
    while let Some(offset) = content[start..].find(PREFIX) {
        let idx = start + offset;
        let before_ok = idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric();
        let end = idx + KEY_LEN;
        if before_ok
            && end <= bytes.len()
            && bytes[idx + PREFIX.len()..end]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && (end == bytes.len() || !bytes[end].is_ascii_alphanumeric())
        {
            return true;
        }
        start = idx + 1;
        if start >= content.len() {
            break;
        }
    }
    false
}

/// `prefix` followed by at least `min_run` alphanumeric/`-`/`_` characters.
fn find_token(content: &str, prefix: &str, min_run: usize) -> bool {
    let mut start = 0;
    while let Some(offset) = content[start..].find(prefix) {
        let idx = start + offset;
        let after = &content[idx + prefix.len()..];
        let run = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .count();
        if run >= min_run {
            return true;
        }
        start = idx + 1;
        if start >= content.len() {
            break;
        }
    }
    false
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_aws_access_key() {
        assert_eq!(
            scan_text("resource: AKIAIOSFODNN7EXAMPLE"),
            Some("AWS access key ID (AKIA...)")
        );
    }

    #[test]
    fn does_not_flag_aws_prefix_as_substring_of_longer_word() {
        assert_eq!(scan_text("AKIATHISISNOTREALLYANAWSKEYBUTLONGER"), None);
    }

    #[test]
    fn detects_private_key_block() {
        assert_eq!(
            scan_text(
                "---\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow...\n-----END RSA PRIVATE KEY-----"
            ),
            Some("PEM private key block")
        );
    }

    #[test]
    fn detects_github_token() {
        assert_eq!(
            scan_text("token: ghp_this_is_fixture_data_not_a_real_token_0000"),
            Some("GitHub access token")
        );
    }

    #[test]
    fn detects_slack_token() {
        assert_eq!(
            scan_text("xoxb-not-a-real-token-just-fixture-data-for-this-test"),
            Some("Slack token")
        );
    }

    #[test]
    fn ordinary_documentation_prose_is_not_flagged() {
        assert_eq!(
            scan_text("# Resetting your password\n\nUse the api_key field to authenticate."),
            None
        );
    }

    #[test]
    fn scan_bundle_finds_a_leaked_key_in_a_generated_concept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("leaky.md"),
            "---\ntype: Concept\ntitle: Leaky\ndescription: AKIAIOSFODNN7EXAMPLE\n---\n\nbody",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("clean.md"),
            "---\ntype: Concept\ntitle: Clean\n---\n\nbody",
        )
        .unwrap();

        let findings = scan_bundle(dir.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, "leaky.md");
    }
}
