# Releasing the plugin to the DITA-OT plugin registry

A release is a `vX.Y.Z` tag on `main`. The plugin, the Rust crates and
the registry entry share that version. The d2g vocabulary has its own
`d2g-X.Y.Z` tags and its own release process
([`semantic-model.md`](../semantic-model.md)).

## What a release ships

`.github/workflows/package.yml` builds `io.github.jyjeanne.dita2graph.zip`:
the plugin directory without `java/`, the licences, and a
`dita2graph-core` build for each platform in `bin/<os>-<arch>/`:

| Directory | Built on | Run in CI |
|---|---|---|
| `linux-x86_64` | `ubuntu-latest`, cargo-zigbuild, glibc 2.28 | yes |
| `linux-aarch64` | `ubuntu-latest`, cargo-zigbuild, glibc 2.28 | yes, on `ubuntu-24.04-arm` |
| `macos-x86_64` | `macos-latest`, cross-compiled | no |
| `macos-aarch64` | `macos-latest` | yes |
| `windows-x86_64` | `windows-latest`, MSVC | yes |

The binaries use the `dist` Cargo profile (stripped, LTO): on Windows
it takes `dita2graph-core` from 22 MB to 9 MB. `ExtractTask` runs the binary for the current
platform (`lib/README.md`) and makes it executable first, because `dita
install` extracts the ZIP with `java.util.zip`, which drops the Unix
permission bits. The workflow then runs `dita install` on the ZIP in a
stock DITA-OT 4.4.1 on Java 17 on Linux (x86_64 and aarch64), macOS and
Windows. Each run builds `sample-docs/` with no other `dita2graph-core`
on the machine.

The jar is compiled with `--release 17` because DITA-OT 4.x runs on
Java 17.

`package.yml` also runs on every push to `main` that touches the
plugin, the core or the build, so a broken package shows up before a
tag does.

## Steps

1. On a branch, set the new version in three places and merge it to
   `main`:
   - `Cargo.toml`, `[workspace.package].version`, then run `cargo build`
     to update `Cargo.lock`;
   - `plugin/io.github.jyjeanne.dita2graph/plugin.xml`, the `version`
     attribute;
   - `io.github.jyjeanne.dita2graph.json`: a new entry at the end of
     the array, with `name`, `description`, `keywords`, `homepage`,
     `vers`, `license` and `deps`, without `url` or `cksum`.
2. Run the **Tag release** workflow (`tag.yml`) from `main` with the
   version. It checks that all three places agree, pushes the tag and
   starts **Release** (`release.yml`).
3. **Release** runs the tests, then `package.yml`, and publishes the
   GitHub release with these assets:
   - `io.github.jyjeanne.dita2graph-X.Y.Z.zip` and its `.sha256`;
   - `io.github.jyjeanne.dita2graph.json`, this version's registry entry
     with its `url` and `cksum` filled in (also shown in the run
     summary);
   - `dita2graph-core` and `dita2graph-mcp` for each platform.
4. Submit the entry to the registry:
   - in a fork of [dita-ot/registry](https://github.com/dita-ot/registry),
     on a branch, add the entry to `io.github.jyjeanne.dita2graph.json`.
     Create the file the first time, as a one-element array. For later
     versions, append the entry and keep the earlier ones;
   - commit with `git commit -s`: the registry requires the DCO
     sign-off;
   - open one pull request per plugin version. The registry's CI checks
     that the file name matches `name`, that `vers` and `deps` are
     semver, and that the SHA-256 of the file at `url` matches `cksum`.
5. Before the pull request, you can test the entry with a custom
   registry. Add the raw URL of the fork's branch in front of the
   official one in `registry` in DITA-OT's
   `config/configuration.properties`, then run
   `dita install io.github.jyjeanne.dita2graph`.
6. Copy the published `url` and `cksum` into the entry in this
   repository's `io.github.jyjeanne.dita2graph.json`, so it stays the
   record of every published version.

A published ZIP never changes: the registry checks its checksum. A fix
means a new patch version.
