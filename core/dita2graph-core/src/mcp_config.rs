//! Writes `mcp/mcp-server.toml` (`docs/plugin-specification.md` §2.3's
//! `args.dita2graph.mcp`, §2.4, §5.4) -- a minimal, real config
//! `dita2graph-mcp --config` can read, not a decorative file nothing
//! consumes. Deliberately doesn't also write a `manifest.json`: §5.1's
//! resource list (`dita://topics`, etc.) isn't implemented by
//! `dita2graph-mcp` -- only `tools/list`/`tools/call` are (§5.2) -- so
//! declaring resources there would describe a capability that doesn't
//! exist. `store`/`graph.db` are left out for the same reason (§7: not
//! implemented).

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Writes `<output_dir>/mcp/mcp-server.toml`. Its `graph.okf` value is
/// `../okf`, relative to the `mcp/` directory the file itself lives in
/// (a sibling of `okf/` under `output_dir`, §2.4) -- `dita2graph-mcp
/// --config` resolves that back to the bundle root by taking its
/// parent directory.
pub fn write_mcp_config(output_dir: &Path) -> Result<()> {
    let mcp_dir = output_dir.join("mcp");
    fs::create_dir_all(&mcp_dir).context("creating mcp/")?;
    let content =
        "[server]\nname = \"dita2graph\"\ntransport = \"stdio\"\n\n[graph]\nokf = \"../okf\"\n";
    fs::write(mcp_dir.join("mcp-server.toml"), content).context("writing mcp/mcp-server.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_config_dita2graph_mcp_can_parse_back() {
        let dir = tempfile::tempdir().unwrap();
        write_mcp_config(dir.path()).unwrap();

        let raw = fs::read_to_string(dir.path().join("mcp/mcp-server.toml")).unwrap();
        let parsed: toml::Value = raw.parse().unwrap();
        assert_eq!(parsed["server"]["name"].as_str(), Some("dita2graph"));
        assert_eq!(parsed["server"]["transport"].as_str(), Some("stdio"));
        assert_eq!(parsed["graph"]["okf"].as_str(), Some("../okf"));
    }

    #[test]
    fn the_okf_path_resolves_to_the_bundle_root_via_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        write_mcp_config(dir.path()).unwrap();
        fs::create_dir_all(dir.path().join("okf")).unwrap();

        let config_path = dir.path().join("mcp/mcp-server.toml");
        let config_dir = config_path.parent().unwrap();
        // Mirrors dita2graph-mcp's own resolution: join the configured
        // okf path onto the config file's directory, canonicalize it
        // (a plain lexical .parent() on an unresolved "mcp/../okf"
        // would incorrectly yield ".../mcp/.." instead of collapsing
        // the ".." -- confirmed by trying it before adding this),
        // then take *that* path's parent to get the bundle root
        // BundleReader expects.
        let okf_path = config_dir.join("../okf").canonicalize().unwrap();
        assert_eq!(
            okf_path.parent().unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }
}
