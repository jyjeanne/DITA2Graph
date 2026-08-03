# sample-docs-invalid

A deliberately broken fixture for the validation-gate guarantee in
`docs/plugin-specification.md` §9.3/§10: a source map never produces a
stale or partially-built graph, because DITA-OT's own validation rejects
it first.

`broken.ditamap` → `topics/broken-topic.dita` contains an `<xref
href="does-not-exist.dita"/>` to a file that doesn't exist. Confirmed
directly (not assumed) which kind of brokenness actually fails DITA-OT's
build, since not every "broken" reference does:

- An unresolvable **`keyref`** (e.g. `<xref keyref="does-not-exist"/>`)
  is only **informational** by default — DITA-OT logs `[DOTJ047I]
  Unable to find key definition ... Using the @href attribute as
  fallback if it exists` and the build succeeds, just with the xref's
  target silently dropped. It does **not** fail validation.
- An unresolvable **`href`** to a nonexistent file *does* fail, with a
  real error: `[DOTX008E] The resource '...' cannot be loaded` and a
  non-zero exit code. This fixture uses that.

Exercised by `gradle-build/build.gradle.kts`'s `validateBrokenDoc` task,
which is expected to **fail** — see that project's README for how it's
wired into CI as a "this must fail" check rather than a normal build
dependency.
