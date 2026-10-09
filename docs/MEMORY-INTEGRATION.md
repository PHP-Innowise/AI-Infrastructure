# Shared memory in Claude Code, Codex and Cursor

Harness and direct clients use the consuming project's existing `project-brain/`
and `memory-bank/`. Install the accelerator in the actual worktree. SQLite and
local receipts are disposable; shared records/chunks remain authoritative.
Memory MCP needs only Python 3.9+ and SQLite FTS5, with no external service or key.

## Harness automatic sessions

Each technical message receives a bounded capsule. Automatic hooks and Harness
cut recognized secrets, email/phone/customer-identifier spans and transcript
role prefixes (`user:`, `stderr:`) out of the message before constructing memory
queries or automatic task goals; the words around them stay, because pasted logs
and addresses are ordinary in real prompts. A private key goes with its body.
For labelled names/addresses the adapter keeps only preceding text, since later
delimiters may belong to the value. A message with nothing technical left skips
retrieval rather than falling back to the task goal or branch name. The original
user message still goes to the native agent. Direct CLI/MCP queries and shared
writes keep strict validation. Hooks and Harness deliver the same rendered text:
selected excerpts within 3,600 characters (`capsule_text` in `refresh --json`).

If the final reply has no parseable
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
The installer checks Python 3.9+ and SQLite FTS5 on the client PATH, including
`python` / `py -3` on native Windows, and preserves other servers/settings.

- Claude Code: `.mcp.json` uses a Python launcher anchored by the server-process
  `CLAUDE_PROJECT_DIR`, with a nearest-project-marker fallback.
- Codex: `.codex/config.toml` carries one managed `mcp_servers.harness_memory`
  table. The Python launcher finds the nearest declaring config from the
  session directory, including nested directories and worktrees.
- Cursor: `.cursor/mcp.json` uses `${workspaceFolder}`.

Launchers stop at their first project marker. Missing runtime there fails rather
than using an ancestor project's memory. Paths are portable across clones and
worktrees; registration never writes global client configuration.

A foreign server using the memory name, malformed configuration, symlinks and
edited managed Codex blocks block registration rather than overwriting them.
On Python 3.9/3.10, Codex merges validate a conservative TOML subset and refuse
unsupported syntax. Run registration with Python 3.11+ for the full TOML parser.
Dry runs and collision preflight write no project files. Harness setup previews
these merges while omitting other servers' configuration values.

For an already installed project after updating the runtime, the same helper can
repair registration in one command (select only clients used by that project):

```bash
python3 memory-bank/scripts/mcp_config.py --root . --tool claude --tool codex --tool cursor --dry-run
python3 memory-bank/scripts/mcp_config.py --root . --tool claude --tool codex --tool cursor
```

Native clients still control workspace trust and MCP tool approval. Accept project
MCP in Claude's `/mcp`, trust the Codex project, and enable the Cursor project MCP.
Allow the four memory tools through the client's supported approval settings when
automatic calls are desired. Configuration alone is not a grant of trust, and
registration does not force the agent to use every retrieved claim.
See [Claude MCP](https://code.claude.com/docs/en/mcp),
[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli), and
[Cursor MCP](https://cursor.com/docs/mcp).

## Four tools

- `memory_status`: check installed memory health.
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
  Records explicitly say **agent-attested, not human-reviewed**. Empty learnings
  is valid; explicit `next_steps: []` clears finished steps.

Use `CONTEXT_TASK_ID`, otherwise the branch task. On detached HEAD, use an explicit
existing task ID. Do not create another task for the same work. Saving runs promotion
under the existing setting. Shared intent/completion events prevent blind retries
and duplicate results after restart/cache deletion or from another worktree.
Different content under an existing result ID, stale revisions, unauthorized actors,
private sources and ambiguous partial saves are refused. Inspect partial records
before reconciling. Completion remains explicit.

Without MCP use the installed context CLI and Project Brain protocol. `/memory`
refreshes layers; `/checkpoint` saves working continuity; reusable knowledge needs
source-backed findings/decisions. This does not authorize unrelated skill chaining
or writes outside a read-only scope.

## Verify

Confirm all four tools appear in each configured client. Retrieve for a fresh task,
checkpoint with a goal, record a source-backed result, then replay the same ID/content
and confirm no duplicates. Inspect Harness **Knowledge → Memory use**, run
`context.py validate --json` and `memory-bank/scripts/validate.py`. Distinguish real
client connections from isolated protocol tests.
