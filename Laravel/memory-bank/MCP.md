# Shared memory in Claude Code, Codex and Cursor

Harness and direct clients use the consuming project's existing `project-brain/`
and `memory-bank/`. Install the accelerator in the actual worktree. SQLite's
search index is rebuildable, while unfinished turn observations must be retained;
shared records/chunks remain authoritative and receipt loss does not authorize a new save.
Memory MCP uses the installed native engine with bundled SQLite FTS5, with no external service or key.

## Harness automatic sessions

Each message receives a bounded capsule. If the final reply has no parseable
`memory-draft`, Harness makes one internal draft-only follow-up in the same native
session, in plan/read-only mode, without helpers or a new user message. Its limit
is 30 seconds within the original time/token/USD budget. Missing native identity
or unmeasurable remaining usage produces an explicit skip. Usage and instruction
character counts include recovery. `CONTEXT_MEMORY_RECOVERY=1` suppresses memory
hooks. Recovery failure preserves completed work; cancellation keeps cancelled
status and prevents subsequent writes. An already launched write may finish.
Reviewed sessions/native Claude commands keep their flow.

Optional `used_memory` lists up to ten delivered paths whose claims the agent
checked and used. The notice distinguishes delivered pointers from **agent-reported
use**; neither proves reading. Telemetry stores counts, not source bodies/path lists.

## Automatic project registration

The accelerator installer and generator register Memory MCP for each selected
client. No separate `claude mcp add` command or manual config entry is needed.
The installer selects the verified native engine for the platform and preserves
other servers and settings. SQLite FTS5 is part of the native runtime.

- Claude Code: `.mcp.json` contains one managed memory server.
- Codex: `.codex/config.toml` contains one managed `mcp_servers.harness_memory`
  table, parsed and merged natively.
- Cursor: `.cursor/mcp.json` contains one managed memory server.

On POSIX each entry names no path of the installing machine: a small `sh -c`
launcher finds this checkout's `memory-bank/bin/infra-engine` through
`CLAUDE_PROJECT_DIR` (set by Claude Code in the server's environment),
Cursor's `${workspaceFolder}` or Git's top level (Codex), and runs
`mcp --root CHECKOUT --host HOST`; a linked worktree uses its main checkout's
engine. Every clone works once its own engine is installed. On native Windows
each entry invokes the absolute `memory-bank/bin/infra-engine.exe` with
`mcp --root FINAL_PROJECT --host HOST`, and moving the installation requires
native re-registration for its new root. Generation can stage elsewhere while
registering the eventual delivery root. Source templates are materialized
before use; there is no ancestor-project fallback or interpreter bootstrap.
Registration writes only project configuration.

A foreign server using the managed memory name, malformed configuration,
symlinks and edited managed Codex blocks block registration rather than
overwriting them.
Dry runs and collision preflight write no project files. Harness setup previews
these merges while omitting other servers' configuration values.

For an already installed project after updating the runtime, the same helper can
repair registration in one command (select only clients used by that project):

```bash
infra-engine repo mcp-config --root . --tool claude --tool codex --tool cursor --dry-run
infra-engine repo mcp-config --root . --tool claude --tool codex --tool cursor
```

Native clients still control workspace trust and MCP tool approval. Accept project
MCP in Claude's `/mcp`, trust the Codex project, and enable the Cursor project MCP.
Allow the five memory tools through the client's supported approval settings when
automatic calls are desired. Configuration alone is not a grant of trust, and
registration does not force the agent to use every retrieved claim.
See [Claude MCP](https://code.claude.com/docs/en/mcp),
[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli), and
[Cursor MCP](https://cursor.com/docs/mcp).

## Five tools

- `memory_status`: check installed memory health.
- `memory_source_fingerprints`: read the complete checked canonical file image
  for 1–10 `sources`, without creating a task or attesting truth. Text hashes
  normalize CRLF to LF. Use these pins only after checking the actual claim.
- `memory_retrieve`: pass the current `task_id` and a concise module/class/ticket
  `query` before complex work, after compaction/scope changes and before uncovered
  decisions. Optional `paths` link canonical sources. Verify cited claims. The
  usual privacy/freshness/budget limits apply, with ignored local manifests.
  A new task receives a warming capsule without an authoritative write.
- `memory_checkpoint`: reuse the same task ID; supply `goal` when it is new.
  It provisions/binds that task even before any edit, flushes Git metadata and
  returns the authoritative numeric revision.
- `memory_record_result`: pass the current `revision`, a stable `result_id`,
  sanitized `progress`, up to three `next_steps`, and up to three reusable
  findings/decisions. Each learning needs `type`, `title`, `consequence` and
  1–10 canonical project `sources`. Set `verified: true` after checking claims.
  To explicitly confirm/correct an existing claim, retain its exact type, title
  and source set and supply all of `record_id` (Brain UUID), `expected_revision`
  and the complete checked `source_fingerprints`. A protected/mismatched target
  is held, never recreated. An identical checked image is a record no-op;
  changed checked evidence versions the existing claim and its promoted chunk.
  Records explicitly say **agent-attested, not human-reviewed**: the verification's
  ledger entry carries `[attestation:agent]`, a chunk promoted from it is tagged
  `agent-attested`, and the capsule marks such knowledge where it delivers it.
  Empty learnings is valid; explicit `next_steps: []` clears finished steps.
  Where the Stop hook asks for the `native-result` save - governed Claude Code,
  Codex and Cursor sessions running the hooks, outside Harness sessions, unless
  `native_result_request` is `off` - that save records the answer: do not
  record the same result with this tool or `record-result` as well.

Use `CONTEXT_TASK_ID`, otherwise the branch task; in a worktree Codex made, on a
detached HEAD, `codex-thread/<thread ID>`, which the capsule names. On any other
detached HEAD, use an explicit
existing task ID. Do not create another task for the same work. Saving runs promotion
under the existing setting. A learning's record is named by its content, so a retry,
or a later result restating it, finds that record instead of writing a second one.
A shared metadata-only result-attempt fence precedes record changes. Exact
completed proof permits passive replay and preserves later task/learning edits,
even when the disposable local receipt is missing. A new v2 started attempt
resumes at its exact positively observed record/task frontier when source,
policy, owner and runtime scope are unchanged. Any other interrupted attempt with
the same result ID, digest, owner and attestation runs again from the start:
a learning is created only when absent, finished only when it is exactly this
save's open record, revised only under the usual proofs, and the task takes only
progress it does not already hold. Losing both proofs does not prove historical
ownership or authorize adoption; preserve result-attempt metadata with the
shared Brain state. It lives in `project-brain/control/result-attempts/<task UUID>/`,
one file per task, result ID and request content: another clone that reuses a
result ID for other content makes its own save, and the two clones' files merge
without a conflict. A finished save keeps only a small proof, the newest 64 per
task; compaction removes an archived task's folder and the
`control/result-learning-images/` of archived learnings. The installer excludes another project's attempts.
This is consistency/application ordering, not authentication or a power-loss transaction.

New writes use the current positive task revision. CLI `--revision auto` captures
it under lock, including tasks already updated by hooks; it never rebases an
ambiguous prior attempt. Unchanged runtime-owned agent knowledge can be reworded
automatically using its exact saved UUID/revision/byte image. A manual/person
change invalidates that proof. Deliberate correction may supply the original
positive learning `expected_revision`; never invent one. A protected, archived,
wrong-kind or conflicting learning is retained individually with revision/reason;
independent progress, next steps and valid facts still save. Stale numeric task
CAS, privacy, source and request-identity violations remain hard refusals.
Without MCP: `memory-bank/bin/infra-engine memory record-result --task-id ID
--result-id RID --revision auto --input result.json`. Harness captures
the native task identity and original safe request automatically; generic stale
errors never authorize rebasing an uncertain retry. Completion remains explicit.

Without MCP use the installed context CLI and Project Brain protocol. `/memory`
refreshes layers; `/checkpoint` saves working continuity; reusable knowledge needs
source-backed findings/decisions. This does not authorize unrelated skill chaining
or writes outside a read-only scope.

## Verify

Confirm all five tools appear in each configured client. Retrieve for a fresh task,
checkpoint with a goal, record a source-backed result, then replay the same ID/content
and confirm no duplicates. Inspect Harness **Knowledge → Memory use**, run
`memory-bank/bin/infra-engine memory validate --json` and `memory-bank/bin/infra-engine bank validate`. Distinguish real
client connections from isolated protocol tests.


## Unicode case-folding data notice

The native runtime includes default full case-folding mappings pinned to Unicode 16.0.0, derived from the Unicode Character Database CaseFolding.txt (https://www.unicode.org/Public/16.0.0/ucd/CaseFolding.txt; SHA-256 6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb). This preserves the canonical Python 3.14 semantic-key contract without depending on a compiler's Unicode version. The Unicode-data license is reproduced in full below and accompanies the native artifact's distributed memory-bank documentation.

UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2026 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
