# Known issues in the vendored bundle

Tracked here (rather than silently worked around) because this is a
vendored third-party build, not code this repo maintains — see
`README.md` in this directory for the vendoring rationale. Fixing any
of these means either updating the vendored copy once upstream fixes
it, or reporting/patching it in
[`jyjeanne/ditacraft`](https://github.com/jyjeanne/ditacraft) directly.

**Currently open:** none.

## Resolved

### 1. Pathological CPU spin on some real-world `<codeblock>` content — fixed in v0.9.1

**Found:** running `validate_live` against every one of the 267 real
`.dita` topics in [`dita-ot/docs`](https://github.com/dita-ot/docs)
(the DITA-OT project's own documentation, chosen as a large, real,
professionally-authored corpus to end-to-end test this feature against
— not a synthetic fixture). 266/267 topics completed in 1.4–2.0s each;
one (`topics/plugins-registry.dita`) reliably burned a full CPU core for
the entire 20s `RESPONSE_TIMEOUT` (confirmed via `ps`: the *vendored
Node process*, not `dita2graph-mcp` itself, at 80–93% CPU for the whole
window) and was killed by that timeout, 100% reproducibly across three
separate runs.

**Root cause, bisected down to the single element responsible** (by
repeatedly truncating the real file to smaller and smaller fragments
and re-running `validate_live` against each with a short cap — not
guessed): this exact `<codeblock>`, alone, in an otherwise-empty topic,
reproduced the hang:

```xml
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
```

Reported upstream as
[`jyjeanne/ditacraft#125`](https://github.com/jyjeanne/ditacraft/issues/125)
with the full bisection trail (this file's own history has the original
version of that trail, if needed).

**Actual root cause, per the upstream fix** (confirming the bisection's
"regex with catastrophic backtracking" hypothesis, more precisely than
this file's own investigation could without `server/src/`): the XXE/
entity-expansion pre-check (`checkEntityExpansion`, run unconditionally
as step 1 of `validateDITADocument` for *every* document — not a
Schematron rule, not something specific to `<codeblock>` handling)
used two backtracking regexes with a classic ReDoS shape: a lazy
`[\s\S]*?` scanning for a DOCTYPE internal subset that, on a DOCTYPE
*without* one, doesn't stop at the DOCTYPE's own `>` and keeps scanning
the rest of the document for the next `[` — which the `<codeblock>`'s
JSON content supplies, handing the subsequent quoted/unquoted
alternation combinatorially many ways to fail to match `]>`.

**Fixed upstream:** `jyjeanne/ditacraft`
[`213548b`](https://github.com/jyjeanne/ditacraft/commit/213548bdbbf8ae7cd75a597613f32818db3d0a76)
(`fix(server): eliminate ReDoS in entity-expansion pre-check (#125)`)
replaced both backtracking regexes with linear, single-pass scanners
that track quote/bracket state by hand instead of backtracking —
immune to this input class by construction, not just faster on this
one input. A same-day follow-up,
[`0c190c3`](https://github.com/jyjeanne/ditacraft/commit/0c190c31cda39734f17168c0571038e5a916ddf8)
(`fix(server): comments in DOCTYPE subset desync the linear ReDoS-fix
scan`), fixed a correctness regression the first fix introduced (an
apostrophe inside an XML comment in the DOCTYPE internal subset could
desync the new scanner's quote-tracking and cause it to silently skip
the rest of the entity-expansion/XXE check) — both landed together in
the `v0.9.1` release (2026-08-19).

**Confirmed fixed here:** re-vendored `dist/lsp-server.js` from the
`v0.9.1` release asset
(`https://github.com/jyjeanne/ditacraft/releases/download/v0.9.1/lsp-server-0.9.1.zip`)
and re-ran the exact same `validate_live` call that used to time out —
now completes in ~1.3s instead of pegging a CPU core for the full 20s.
Also re-ran the full 267-topic `dita-ot/docs` end-to-end pass; see
`README.md`'s version header for exact commit/version and this
directory's git history for the confirming test run. The regression
test that used to assert the *known-bad* timeout
(`validate_file_hangs_on_a_real_codeblock_from_dita_ot_docs`) has been
flipped to `validate_file_no_longer_hangs_on_a_real_codeblock_from_dita_ot_docs`
in `live.rs`, now asserting the fix (no diagnostics, completes in under
5s) and kept permanently as a regression guard — not deleted — so a
future vendored-bundle update can't silently reintroduce this.

**Impact on `dita2graph-mcp` while this was open:** contained, not
catastrophic — worth noting for anyone reading this after the fact.
`RESPONSE_TIMEOUT` (`live.rs`) killed the child and returned a normal
`isError: true` tool result instead of hanging the whole MCP server;
the server process itself stayed responsive and correctly served the
remaining topics in the same batch afterward, with no orphaned `node`
process left behind. Exactly the scenario `RESPONSE_TIMEOUT` exists for.
