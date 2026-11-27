# Model benchmark: Haiku, Sonnet and Opus with the MCP server

How well does an agent answer questions about a DITA documentation set when
its only source is `dita2graph-mcp`, and how much does the model matter?
This page gathers the measurements made in November 2026 with Claude Code
and the three Claude 5.5 models (Haiku, Sonnet, Opus), and the changes to
DITA2Graph they led to.

The harness, the questions, the raw answers and the grades live in the demo
project (`dita2graph-demo/bench`), not in this repository.

## In short

| | Haiku | Sonnet | Opus |
|---|---|---|---|
| Relevance checks R1–R9, server as of v0.2.0 | 41.7 % / 40.0 % | 50.6 % | 95.0 % |
| Relevance checks R1–R9, with the four changes | 97.2 % | 98.9 % | 100 % |
| 12 real questions, two corpora, blind grading | 94.0 % | 95.7 % | 95.7 % |
| The same, with conref push modeled (estimate, see below) | 94.8 % | 96.5 % | 96.5 % |
| Cost of the 36 real-question runs | $0.31 | $2.09 | $5.42 |

For reference, Opus with the v0.2.0 server scored 88.2 % on the 12 real
questions, for $7.65.

- **Tools matter more than the model.** With the v0.2.0 server, Haiku answered
  the relevance checks at about 41 %. With four changes to the server, it
  answers them at 97 %, above Opus with the old server (95 %), for about a
  hundredth of the cost.
- **The changes hold on questions they were not designed on.** On 12 real
  questions taken from public issues and forums, graded blind, the three
  models with the changes beat Opus before them (94–96 % against 88 %).
- **Every model gets cheaper and faster.** A complete search and paths taken as
  ids replace series of guesses. On R1–R9, Opus keeps 100 % for half its
  former cost ($2.37 instead of $4.90), in 29 s per question instead of 49.
- **Which model to use.** Haiku is close to the other two for a seventh of
  Sonnet's cost and a seventeenth of Opus's. Sonnet is the fastest and uses the
  fewest tokens. Opus gives the most thorough answers.

## Setup

- **Client:** Claude Code 2.1.332, one fresh non-interactive session per
  question. The agent has the `dita2graph` MCP server and nothing else: no file,
  shell, web or subagent tool. The real-question runs use effort `xhigh`.
- **Corpora:**
  - DITA-OT 4.4.1 documentation (`dita-ot/docs`): 225 topics, 623 edges.
  - Oxygen XML Editor user guide (`oxygenxml/userguide`), filtered with
    `editor-sa.ditaval`: 2,044 topics, 10,097 edges.
- **Grading:** 0 to 100 per answer, following the demo's `bench/GRADING.md`.
  - 100 is complete, right and grounded in the corpus; 0 is wrong or contrary
    to the corpus.
  - A list scores the share of expected items found, minus 10 per item or claim
    made wrongly.
  - A question with several repetitions scores the median of its repetitions.
- **Cost and tokens:** as Claude Code reports them for each session.

## Part 1: relevance checks R1–R9

Nine checks on the DITA-OT documentation, each built around a trap that a
plausible but wrong answer falls into.

| Case | What it tests |
|---|---|
| R1 | A topic reuses one fragment of a file, not the whole file |
| R2 | A false positive from shared reuse |
| R3 | Pages that show a command without linking to it (10 pages): what a link graph cannot see |
| R4 | One source file published as two pages under other names |
| R5 | Where to change a URL that a key defines |
| R6 | An inconsistent spelling in the published text (`plugindirs` / `pluginsdir`) |
| R7 | Notes for the authors (`draft-comment`): certain or guess |
| R8 | Version history |
| R9 | A subject the corpus does not cover |

### Four changes, one at a time

The R1–R9 failures of Haiku with the v0.2.0 server led to four changes, each
measured with Haiku before the next one:

| Change | Commit | What it does |
|---|---|---|
| 1. Server instructions | `e289956` | `dita2graph-mcp` returns `instructions` at `initialize`: which documentation the graph holds, answer from the tools, say when the documentation does not cover a subject. The demo's session folders add an `AGENTS.md` (any vendor) and a `CLAUDE.md` |
| 2. Exact, exhaustive search | `a4be64c` | `find_text`: every topic holding a text, literally and with no cap, with the reused passage it comes from, the keys that define it and the near spellings |
| 4. Notes for the authors marked | `035464d` | `[draft-comment]` … `[end draft-comment]` in a topic's text: readers do not see these by default |
| 3. Paths and keys as ids | `149f095` | every tool that takes a `topicId` also takes a source file path or a key name, and says what it resolved to |

### Results

| Setup | Haiku | Sonnet | Opus |
|---|---|---|---|
| Server as of v0.2.0 | 41.7 % / 40.0 % (2 runs) | 50.6 % | 95.0 % |
| No MCP server, file tools on the DITA sources | 79.4 % | | |
| Server as of v0.2.0, plus file tools | 71.7 % | | |
| Change 1 | 88.3 % / 71.7 % (2 runs) | 78.9 % | 100 % |
| Change 1, server instructions only (no `AGENTS.md`) | 82.2 % | | |
| Changes 1 and 2 | 82.2 % / 81.1 % / 82.2 % (3 runs) | 88.3 % | |
| Changes 1, 2 and 4 | 92.2 % | | |
| **All four changes** | **97.2 %** | **98.9 %** | **100 %** |

| All four changes, 9 questions | Haiku | Sonnet | Opus |
|---|---|---|---|
| Time per question | 12.7 s (14.6 s before) | 9.6 s (12.4 s) | 29.0 s (48.6 s) |
| Tokens processed | 0.91 M (1.33 M) | 0.71 M (0.93 M) | 1.29 M (4.22 M) |
| Cost | $0.05 ($0.06) | $0.62 ($0.80) | $2.37 ($4.90) |

What each change settled:

- **Change 1:** no answer comes from memory any more. Before it, Haiku read
  R6 as a question about its own client's settings and answered R8 and R9
  from general knowledge. Haiku went from about 41 % to about 80 %, and the
  server's instructions alone give most of that gain (82.2 %): every MCP client
  receives them.
- **Change 2:** settled R3 and R6. Wording alone did not: a ranked search that
  stops at 15 hits cannot list every page, nor meet a spelling it was not asked
  for.
- **Change 4:** settled R7. Asking for care did not; marking the notes in the
  data did.
- **Change 3:** settled R4. The instruction "a file path is not a topic id" did
  not; taking the path as an id did.

Every case that stayed wrong after change 1 needed a tool or a data change,
not more prompting. Two changes brought new failures, which re-running the
checks caught: a literal match that hid the key defining a URL (R5), and a
complete search with no hit, read as "no" (R7).

These figures come from one to three runs per setup, graded by the session
that ran them against the expected answers (not blind). The changes were
designed on these nine cases, so Part 2 is the real test.

## Part 2: twelve real questions, graded blind

Twelve questions taken from public issues, pull requests and forum threads,
six per corpus, half from users and half from technical writers. None of
them was used to design the changes.

| Id | Audience | Question, in short |
|---|---|---|
| r-d1 | User | A build fails with `DOTA015F` "internal property … may not be set" with a third-party plug-in |
| r-d2 | User | HTML5 output: `args.css`, `args.cssroot`, `args.copycss` and `args.csspath` do not give the expected result |
| r-d3 | User | How to pass several DITAVAL files on the `dita` command line |
| r-d6 | Writer | `outputFile.base` is deprecated: which published topics still show it, sample files included |
| r-d7 | Writer | A broken HTML Help Workshop download link: which pages show it, and how it is defined |
| r-d10 | Writer | An Ant example uses the deprecated `if` attribute: is it fixed everywhere? |
| r-o1 | User | Oxygen keeps validating against an old version of a DTD |
| r-o3 | User | Renaming an element's ID and updating every reference to it |
| r-o4 | User | Keeping custom framework and validation settings across an upgrade |
| r-o8 | Writer | Which topics link to the Editor EULA, and how |
| r-o9 | Writer | What a rewrite and a rename of the license unregistering topic affects |
| r-o10 | Writer | Through which key and file the reused editor variable descriptions are managed |

The runs:

- 3 repetitions per question and model, effort `xhigh`, server at `149f095`
  (all four changes): 108 runs.
- Grading: one blind grader per question, a separate agent with no access to
  the runs. It scored the nine new answers together with the three answers Opus
  gave with the v0.2.0 server, all shuffled under random ids.
- Consistency check: regraded this way, Opus's v0.2.0 answers got exactly their
  earlier averages back.

| Median of 3 repetitions | Opus, v0.2.0 server | Haiku | Sonnet | Opus |
|---|---|---|---|---|
| r-d1 | 100 | 100 | 100 | 100 |
| r-d2 | 100 | 100 | 100 | 100 |
| r-d3 | 100 | 100 | 100 | 100 |
| r-d6 | 100 | 90 | 90 | 90 |
| r-d7 | 100 | 100 | 100 | 100 |
| r-d10 | 75 | 75 | 75 | 75 |
| r-o1 | 75 | 90 | 100 | 100 |
| r-o3 | 25 | 100 | 100 | 100 |
| r-o4 | 100 | 90 | 100 | 100 |
| r-o8 | 100 | 100 | 100 | 100 |
| r-o9 | 83 | 83 | 83 | 83 |
| r-o10 | 100 | 100 | 100 | 100 |
| **Users' questions** | 83.3 | 96.7 | **100** | **100** |
| **Writers' questions** | 93.0 | 91.3 | 91.3 | 91.3 |
| DITA-OT | 95.8 | 94.2 | 94.2 | 94.2 |
| Oxygen | 80.5 | 93.8 | 97.2 | 97.2 |
| **All** | 88.2 | **94.0** | **95.7** | **95.7** |

| Sums over 6 questions of the median per question | Opus, v0.2.0 server | Haiku | Sonnet | Opus |
|---|---|---|---|---|
| DITA-OT: time · tokens · tool calls | 176 s · 744 k · 75 | 165 s · 603 k · 78 | 85 s · 321 k · 48 | 145 s · 560 k · 63 |
| Oxygen: time · tokens · tool calls | 174 s · 563 k · 75 | 201 s · 958 k · 114 | 101 s · 448 k · 69 | 156 s · 549 k · 76 |
| Cost of the 18 runs, DITA-OT · Oxygen | $4.31 · $3.34 | $0.14 · $0.17 | $0.92 · $1.17 | $2.69 · $2.73 |

- **Gains on users' questions (83 % → 97–100 %).**
  - **r-o3** goes from 25 to 100: the answers find the topic on renaming IDs.
    The gain comes mostly from the word search of `search_topics`
    (`b173720`), made after v0.2.0.
  - **r-o1** goes from 75 to 90–100: the answers now name the option that
    caches schemas.
- **Haiku's gap** (94.0 % against 95.7 %) comes from two Oxygen users'
  questions where it leaves out a secondary element: where to find Validate
  (r-o1), and the section on upgrading in one run out of three (r-o4).
- **Ceilings.** r-d10 (75) and r-o9 (83) stay where they were for every model.
  What is missing is not in the published corpus. For r-d10, it is the fact
  that the upstream fix (pull request #630) corrected only one copy of the
  example. For r-o9, it is a map that the DITAVAL filter removes.
- **One drop: r-d6, 100 → 90 for all three models.** The answers had all
  six expected elements, but said that the deprecated entry of the
  resource-only Ant parameter details reaches no reader. Some also called a
  link through the key `parameters-pdf` empty. Part 3 fixes it.

## Part 3: conref push (r-d6)

The DITA-OT documentation pushes its parameter descriptions with conref push
(`conaction="mark"` and `pushafter`) into pages such as `parameters-pdf.dita`.
Its own build generates these pages before running DITA-OT, so they are
absent from a plain build. The graph did not model conref push, and the key
`parameters-pdf` had no target.

Commit `faab990` changes three things:

- **Extractor:** it reads `@conaction` from the source and marks each pushed
  passage in the pushing topic's text, between `[pushed after <file>#<element>]`
  and `[end pushed]`.
- **Graph:** a file the build does not have becomes a `Missing File` node. The
  node is the target of the key that names it, and `generated-from` the topic
  that pushes into it.
- **MCP server:** it marks such files `[not in this build]` and explains what
  they are.

Details: [plugin-specification.md](plugin-specification.md) and
[semantic-model.md](semantic-model.md#keys-external-links-and-sample-files).

r-d6 was run again with this change: 3 repetitions per model, then a blind
grader scored 21 shuffled answers. These were the 9 new ones, the 9 from
Part 2 and Opus's 3 from v0.2.0. The 12 older answers got exactly their
earlier scores back.

| r-d6, scores of 3 repetitions (median) | Haiku | Sonnet | Opus |
|---|---|---|---|
| All four changes | 100 · 90 · 90 (90) | 90 · 90 · 90 (90) | 90 · 90 · 90 (90) |
| With conref push modeled | 90 · 100 · 100 (**100**) | 100 · 100 · 100 (**100**) | 100 · 100 · 100 (**100**) |

Eight answers out of nine now say that the entry is pushed into the
generated PDF parameters page, and none calls the link empty. The remaining
−10 (Haiku, first run) is about something else: the answer counts the
parameter details among the pages to fix.

Only r-d6 was run again. If the other eleven questions keep their scores, the
averages become:

- writers' questions: 93.0 % for all three models;
- all questions: Haiku 94.8 %, Sonnet and Opus 96.5 %.

## Limits

- **Few runs.** R1–R9: one to three runs per setup. A 10-point gap on one case
  is not significant: one Haiku case moved from 75 to 0 between two identical
  runs.
- **Who graded.** R1–R9 were graded by the session that ran them. The real
  questions were graded blind, but by agents, not people.
- **One client.** Every run used Claude Code with Claude models. The `AGENTS.md`
  files are meant for other agents too, but no other client or vendor was
  measured.
- **Cost.** It depends on prompt caching and on the prices at the time. The
  ratios between models matter more than the amounts.
