# lib/dita2graph-core.jar

Not yet built. This is where the thin Java bridge (§2.1, §3.1) goes once
Phase 1 gets to actually installing and running against DITA-OT: a small
Ant task (`org.dita.dita2graph.tasks.ExtractTask`, referenced from
`../build.xml`) that reads DITA-OT's resolved `job.xml` and temp-directory
topics/maps, serializes the normalized DITA model (§3.2), and shells out
to the `dita2graph-core` binary (already implemented, see
`../../../core/dita2graph-core`) to do the rest.

Nothing in `core/` or `mcp/` depends on this jar existing — both build and
test standalone via `cargo build`/`cargo test` from the repo root. This
jar is only needed to wire the Java-side DITA-OT hook (§2.2) up to that
already-working Rust pipeline.

Target/source: **Java 25 (LTS)**, matching `docs/plugin-specification.md`
§1.1 and the repo-root `.java-version` file. DITA-OT 4.4 itself only
requires Java 17+, so this is a floor DITA2Graph sets for its own code,
not a DITA-OT requirement.
