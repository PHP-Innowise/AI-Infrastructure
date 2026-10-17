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
or unmeasurable remaining usage produces an explicit skip. Usage, instruction
character counts and the context fill include recovery. `CONTEXT_MEMORY_RECOVERY=1` suppresses memory
hooks. Recovery failure preserves completed work; cancellation keeps cancelled
status and prevents subsequent writes. An already launched write may finish.
Reviewed sessions/native Claude commands keep their flow.

Optional `used_memory` lists up to ten delivered paths whose claims the agent
checked and used. The notice puts the delivered items beside those the run's own
tools were seen to open (a Read that returned the file or any tool's copy of a
skill, a Skill load; *at least* that many when shell commands also ran, *unknown*
when they were the only way the run read, as for Codex) and beside **agent-reported
use**; neither proves a claim was used. Telemetry stores counts, not source
bodies/path lists.

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

Cursor's installed hooks use a persistent-rule delivery profile. Short replies
such as `yes do it`, `да`, and `fix it` may use the last safe substantive query
for the same current project, named branch, task generation and policy. The
ignored SQLite row stores only that query and expires after one day; every
delivery refreshes and verifies current sources. A new explicit goal wins over
the old query. Changed scope, redaction, incomplete refresh or unknown ownership
withholds the query rather than reusing old facts. The first automatic checkpoint
can transfer it only to the task that checkpoint actually created.

Routine checkpoint/promotion counts do not rewrite the Cursor rule. Current
progress, next steps, source changes and failures still change its substantive
content. The ordinary CLI, Harness, Claude and Codex delivery profiles retain
their existing history and repeat behavior. This is local delivery behavior;
it does not establish model quality or provider-token savings.

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
  Records explicitly say **agent-attested, not human-reviewed**: the verification's
  ledger entry carries `[attestation:agent]`, a chunk promoted from it is tagged
  `agent-attested`, and the capsule marks such knowledge where it delivers it.
  Empty learnings is valid; explicit `next_steps: []` clears finished steps.

An automatic promotion attempt schedules its own retry before doing work.
If it fails or is interrupted, the next governed turn after the 30-second
delay retries one record, even when Git is clean. Owner-scoped metadata lives
under ignored `project-brain/local/`; it contains a timestamp and a record UUID,
never learning text. Attempts rotate so one failing source cannot starve other
records. Disabled promotion and lightweight mode remain inactive. A successful
scoped save cannot discard another record's pending work. Scheduling must be
writable before a background retry scans sources; explicit promotion can still
save records when this optional local state is unavailable. These retries need
subsequent turns; they are not a background timer. Older failures without a
marker are recovered by the existing normal flush or exact result replay.

Use `CONTEXT_TASK_ID`, otherwise the branch task. During a rebase that is the
branch being rebased: the prompt hook reads its memory and the Stop hook
checkpoints that same task while the rebase runs.
On any other detached HEAD, use an explicit existing task ID. Do not create
another task for the same work. Saving runs promotion
under the existing setting. A learning's record is named by its content, so a retry,
or a later result restating it, finds that record instead of writing a second one.
An exact completed result replays passively and preserves later edits. A
metadata-only fence in `project-brain/control/result-attempts/<task UUID>/` is
saved before record changes and completed before the disposable local receipt;
its file is named by task, result ID and request content, so another clone that
reuses a result ID for other content makes its own save and the clones' files
merge without a conflict. Losing only the receipt still permits exact replay. A
started attempt continues at a positively observed, unchanged frontier: task
UUID/revision, record byte images, current cited source fingerprints,
policy/actor/runtime scope and bounded native record/index/handoff domain must
match. Any other interrupted attempt with the same result ID runs again from the
start, every effect idempotent against the current records: a learning is
created only when absent, finished only when it is exactly this save's open
record, revised only under the usual proofs, and the task takes only progress it
does not already hold. No arbitrary rollback or power-loss transaction is
promised. Preserve these operation proofs with the Brain state; the installer
never copies another project's proofs. A finished save keeps only a small proof;
the newest 64 per task are kept, and compaction removes an archived task's folder
and the `control/result-learning-images/` entries of archived learnings.

Existing learnings outside the public/team save privacy scope are held without
returning their current title, UUID, status or revision. Only normalized incoming
learning fields and a fixed held reason are returned, including archived and
completed replay paths. Promotion still runs normally; inaccessible, missing or
ambiguous record identities are omitted from its response diagnostics with
`details_withheld=true`. This is output filtering, not deletion or skipped work.

CLI `--revision auto` reads the current task revision under the mutation lock
for a fresh request, including tasks already updated by hooks. It does not rebase
an ambiguous existing attempt. A numeric stale task CAS still rejects the new
operation before effects. MCP requires the numeric revision from checkpoint.
Rewording an agent's own closed/verified learning can update automatically only
when its exact UUID/revision/raw bytes match the runtime's last saved image.
Manual changes invalidate that proof even if an older agent attestation remains.
Deliberate correction may supply the original positive learning
`expected_revision`; never invent or reload one to conceal intervening changes.
Person-attested, archived, wrong-kind, unauthorized and conflicting learning
records stay intact and are reported individually with current revision/reason.
Independent progress, next steps and other valid facts still save. Content/path
privacy, result-ID/content identity and task authorization remain hard guards.
Production retrieval remains enabled. CLI example: `python3
memory-bank/scripts/context.py record-result --task-id ID --result-id RID
--revision auto --input result.json`. Completion remains explicit.

The native CLI/MCP accepts exact receipted replay after task archival or citation
deletion. Current task ownership/privacy and static content/path checks still
apply; the replay leaves later corrections and removed records alone. Current
source checks still govern every new result and any promotion retry. Reused
ordinary aliases select the active task; ambiguous UUID-shaped aliases are
refused. Use the task UUID to name an archived task unambiguously. CLI fresh-task or proven replay `auto` resolution occurs under the mutation lock; MCP
requires a positive integer revision. For an original live automatic save with a canonical linked UUID,
Harness now calls `prepare-result`, which validates content/privacy and returns a
versioned safe wire request. An insert-only private table binds that request and
its original omissions to the session, full generation, originating event and
runtime/actor/policy scope before the first mutation. A contender or repeat uses
`replay-result`: exact receipt proof and current authorization are required before
any record or promotion writes. Source deletion and archival do not regenerate
the request. A started attempt with the same digest, owner and attestation is
finished by the replay, whatever stopped it: each effect is idempotent against
the current records. Missing/corrupt proof and changed input/scope remain
incomplete; they never fall back to an ordinary write or the legacy chain.

The binding body is separate from public session/launch rows and cost metadata;
the original provider draft remains in its existing conversation event. Rejected
candidates may pass through the existing short-lived private request file, which
is cleaned on success and failure, but are never retained in the immutable table.
Bindings remain for the lifetime of the private session database. Each binding
has a 128 KiB admission limit; there is no lifetime row-count cap or unresolved-row eviction.
No startup or post-finalization replay is authorized by a persisted binding.
Owned automatic alias-only links resolve and bind their current canonical task
UUID before saving; the original UUID and alias selector remain bound on every repeat. Generic stale
errors never authorize refresh/rebase after an attempt may have started. Manual
saves and unsupported old runtimes do not acquire historical recovery permission. This is persistence correctness, not evidence of provider-token savings.

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

## Automatic results in native sessions

Governed Codex, Claude Code and Cursor sessions now use one bounded same-agent Stop continuation to record the result of the work already performed. The prompt hook arms ignored metadata only; the normal Git checkpoint runs first at Stop. The continuation calls `context.py native-result` with progress, optional next steps and at most three source-backed findings/decisions. `verified` is a top-level boolean and means agent attestation, never human review. Empty learnings and an explicit skip are valid. No prompt, final prose or transcript is parsed into authoritative knowledge.

A read-only research result can provision its task after validation without inventing a Git delta. The challenge pins project, actor, host/session, task UUID/revision, checkout and runtime/config. A new user prompt retires the prior generation; the native internal continuation preserves it. Duplicate Stops request no additional continuation; exact repeated saves reuse existing result fencing. The recovery Stop does not checkpoint the same user work again. Completion still requires the explicit verified workflow.

Challenges live under ignored `project-brain/local/native-results/`, contain metadata only, allow save for 24 hours and retain at most 128 sessions. Missing/invalid state is a refusal, never permission to overwrite. A full save proof without its outcome metadata remains held because it cannot prove every learning was accepted. Failed/held capture is surfaced by the next capsule, including Cursor's JSON-safe rule path. Cursor sets `loop_limit: 1`; aborted/error stops do not capture. Harness-delivered, internal-recovery and lightweight sessions keep their existing behavior.

Hooks need Python and a trusted/enabled native hook configuration. The integration adds one internal model continuation when a native result has not yet been submitted; its cost is not assumed to be free or offset by savings. Live-host acceptance and task/cost evaluation are separate checks.
