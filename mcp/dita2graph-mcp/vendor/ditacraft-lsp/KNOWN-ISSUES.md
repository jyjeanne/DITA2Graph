# Known issues in the vendored bundle

Tracked here (rather than silently worked around) because this is a
vendored third-party build, not code this repo maintains — see
`README.md` in this directory for the vendoring rationale. Fixing any
of these means either updating the vendored copy once upstream fixes
it, or reporting/patching it in
[`jyjeanne/ditacraft`](https://github.com/jyjeanne/ditacraft) directly.

## 1. Pathological CPU spin on some real-world `<codeblock>` content

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
reproduces the hang:

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

Further bisection of the JSON content itself: `xml:space="preserve"` is
*not* required (removing it still hangs). Trimming the block down to
everything *except* the last (`cksum`) line drops it from "still
hanging past 8s" to "completes in ~4.1s" — nearly 3x slower than every
other real topic's ~1.4s baseline for one 90-character line removed.
That shape (small, linear-looking input growth producing disproportionate,
apparently super-linear time growth) is the signature of a regex with
catastrophic backtracking somewhere in the regex-based validation
pipeline that scans raw document text (`docs/DITA_LSP_ARCHITECTURE.md`
in `jyjeanne/ditacraft` documents this design choice explicitly: "Most
features operate on raw document text via regex rather than building a
full AST") — most likely triggered by the density of quoted
`"key": "value"` pairs and/or nested `[`/`{` structure inside a single
`<codeblock>`, not by any single character sequence in isolation
(isolated tests of the `cksum` hash alone, the `>=2.3.0` string alone,
and minimal `[{}]`/nested-bracket skeletons with no long strings all
ran fast — see the session that produced this file for the full
bisection trail).

**Impact on `dita2graph-mcp`:** contained, not catastrophic.
`RESPONSE_TIMEOUT` (`live.rs`) kills the child and returns a normal
`isError: true` tool result ("timed out waiting for ditacraft-lsp to
respond") instead of hanging the whole MCP server; confirmed the server
process itself stayed responsive and correctly served the remaining
topics in the same batch afterward. `validate_file`'s cleanup
(`child.kill()`/`wait()`) leaves no orphaned `node` process behind. This
is exactly the scenario `RESPONSE_TIMEOUT` exists for.

**Not fixed here:** the offending code is inside the minified,
vendored `dist/lsp-server.js`, not TypeScript source this repo owns —
bisecting to a specific regex/line in `jyjeanne/ditacraft`'s
`server/src/` would need that repo's source and its own test harness.

**Reported upstream:**
[`jyjeanne/ditacraft#125`](https://github.com/jyjeanne/ditacraft/issues/125).
Once fixed there and the vendored bundle is updated (see `README.md`'s
"Updating this vendored copy"), re-run
`cargo test -p dita2graph-mcp -- --ignored validate_file_hangs_on_a_real_codeblock_from_dita_ot_docs`
to confirm, then remove this file (or flip it to a changelog entry) and
the `#[ignore]` on that test.
