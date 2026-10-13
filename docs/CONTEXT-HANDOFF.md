# Merge Chats into a New Task

## Harness UI

Harness provides **Sessions → Merge chats** for selecting 2–8 saved, inactive
chats from the same registered project and starting a new Codex, Claude or
Cursor task with their frozen, attributed visible histories. See the
[Harness workflow and limits](../harness/README.md#merge-chats-into-a-new-task).
This selection uses Harness's own message history; native transcript export
and installed-project automatic capture remain separate capabilities below.

For explicitly merged Harness tasks, `CONTEXT_CONTINUITY_RESTORE_DISABLED=1`
disables only native-hook restore. Capture continues, so progress remains
available to subsequent tasks. The flag also accepts `true` or `yes`.

## Automatic Merge with Installed Infrastructure

Trusted project hooks capture visible context for **Codex**, **Claude Code**,
and **Cursor**. When a new task starts in the same checkout and Git branch,
it receives a merge of up to eight recent captured chats, including chats from
other supported clients. This default is automatic: no manual save, paste,
command, or agent delegation is needed. It includes all retained same-branch
sources, which can contain different topics; use selected merging below when
only particular chats belong in the new task.

Each source keeps its own identity, client, commit, capture time, completeness
label, and exact captured text. The new task receives a balanced preview capped
at 6,000 UTF-8 bytes **including headers** (also bounded below the Codex token limit), plus a path to the complete local
source archive. The receiving agent reads omitted source content before using
it, preserves each chat's context, decisions, completed work and next steps,
and reports conflicting decisions by source. The runtime does not semantically
resolve conflicts; they remain `not-evaluated` until the agent checks them.

Sources are copied atomically into `.context-handoff/merges/` and bound to the
new client's task identity. Later updates or rotation of source snapshots do
not alter that merge. Reopening the task restores the same sources plus its
own captured progress. A source chat resumes its own context and does not
consume a merge prepared for another task.

`.context-handoff/` is ignored local state, outside Project Brain, Memory Bank,
SQLite, retrieval indexes, and publishing. Capture retains eight source chats
per branch and at most 4 MiB of visible text per source. Frozen merge archives
survive source rotation until explicitly deleted. Each serialized archive is
limited to 32 MiB including JSON escaping and metadata; eight individually
valid 4 MiB sources can therefore exceed the combined limit. Choose fewer or
smaller sources in that case. The checkout holds at most 128 archives and
256 MiB total.
Exceeding a limit refuses the new merge without deleting existing task history.
Explicit selection reports an error; native hooks quietly skip the new merge.
Delete `.context-handoff/merges/` to remove frozen merges; delete the entire
`.context-handoff/` to remove all local continuity state. Old approvals never
transfer, and current project sources remain authoritative.

### Merge selected chats

Ask the agent to merge particular chats, or use `context-load merge` in Codex
and `/context-load merge` in Claude Code / Cursor. The agent lists available
captured sources and freezes 2–8 chosen IDs for the **next new task** in the
chosen client. The source set is captured immediately, not when that task starts.
One pending selection per branch and destination client is allowed. Existing
pending selections are never overwritten (an identical retry succeeds); a malformed or missing selected
source fails the complete operation. A new task claims its selection once under
the same lock used for capture. Other new tasks use the normal automatic merge.

Agent-facing terminal forms (IDs come from `list`; never guess them):

```bash
python3 memory-bank/scripts/context_continuity.py --root . --host codex --event list
python3 memory-bank/scripts/context_continuity.py --root . --host codex --event merge --source-session codex:CHAT_A --source-session claude:CHAT_B
```

`--host` accepts `codex`, `claude`, or `cursor` and selects the destination.
These forms write only ignored local state and report explicit failures with
exit code 1. Native hook failures remain silent and do not block the task.
The same checkout, branch, and trusted hooks must be used by the receiving task.

The merge engine prepares and delivers context; hooks do not create native UI
tasks. When the user requests creation and the client's agent exposes a supported
task-creation tool, the skill uses it. Otherwise the prepared merge waits for the
next newly opened task; it must not be reported as an already created task.

### Client Lifecycle and Limits

| Client | Automatic capture | Automatic restore | What is available |
| --- | --- | --- | --- |
| Claude Code | `UserPromptSubmit` and `Stop` hooks | `SessionStart` hook | A native visible transcript when the hook supplies it; otherwise the visible prompt and final assistant message. |
| Codex | `UserPromptSubmit` and `Stop` hooks | `SessionStart` hook | A native visible transcript when the hook supplies it; otherwise the visible prompt and final assistant message. |
| Cursor | `beforeSubmitPrompt` and `afterAgentResponse` hooks | `sessionStart` hook | The prompt and response `text` fallbacks; Cursor's session-start `additional_context` delivery is asynchronous. |

The runtime accepts a user-visible `.txt`, `.md`, or `.markdown` export when a
native hook makes one available. A recognised JSONL input is projected to only
visible `user` and `assistant` text; it discards system messages, hidden
reasoning, tool calls, envelopes, and metadata. A projected JSONL snapshot is
explicitly partial, never a full transcript. It never searches client account
directories, private session stores, or internal history files.

Native transcript availability varies by installed client version and lifecycle
event. An unavailable, delayed, partial, or projected transcript is recorded
only as the visible content actually supplied; it is never presented as proof
of a complete live session. Current automated checks use synthetic hook
payloads. They do not prove that a live account delivered every event.

A stable SessionStart identity is required (`session_id` for Codex/Claude,
`conversation_id` for Cursor). Missing identity skips restoration. Cursor Cloud
without `sessionStart` cannot automatically receive a merge.

Set `CONTEXT_CONTINUITY_DISABLED=1` (also `true` or `yes`) to disable capture
and restore for a shell or client process. Missing Git branch identity, an
untrusted or disabled hook, missing runtime, malformed input, oversized
content, or likely secrets all fail closed and quietly: the client turn
continues without stored or restored context.

### Trust and Safety

Enable the hooks only through the client's normal project trust and permission
mechanism. The accelerator does not bypass trust prompts or client policy.
Before relying on a restored excerpt, verify material claims against current
sources and follow the current user's request.

The runtime receives only documented visible prompt/assistant fields or an
explicit visible export. It does not read arbitrary account/session-store
content, hidden reasoning, or tool envelopes, and rejects recognised likely
secrets before saving. Per-session retention is bounded; when it trims older
content, restore marks the omission. Snapshots never restore across a branch
boundary. They do not run embedded commands, switch branches, mutate
Brain/Bank/SQLite, create a native task, or preserve approval for later actions.

Official hook references: [Codex](https://developers.openai.com/codex/hooks),
[Claude Code](https://code.claude.com/docs/en/hooks), and
[Cursor](https://prod.cursor.com/docs/hooks).

## Portable Manual Handoff

Use `context-save` and `context-load` only when context must move to another
machine, checkout, client, or a reviewable task artifact is needed. They share
a portable Markdown format across Codex, Claude Code, and Cursor.

| Client | Save | Load |
| --- | --- | --- |
| Codex | `context-save summary` | `context-load tasks/TASK-001/context-save-20260915T120000Z.md` |
| Claude Code | `/context-save summary` | `/context-load tasks/TASK-001/context-save-20260915T120000Z.md` |
| Cursor | `/context-save summary` | `/context-load tasks/TASK-001/context-save-20260915T120000Z.md` |

These are AI skill/command invocations, not terminal executables. The current
agent prepares the curated payload; it does not delegate context extraction.
The result includes an exact absolute path and a ready-to-copy next-task prompt.
Saving never stages, commits, pushes, or publishes the file.

| Detail | Contents | Maximum authored text |
| --- | --- | --- |
| `summary` | Goal, constraints, decisions, progress, verification, questions, next steps, and relevant files | 8,000 Unicode characters |
| `topic` | The same sections for a named subject | 16,000 Unicode characters |
| `full` | Curated continuation plus an explicitly supplied visible conversation export | 64,000 authored characters plus a bounded transcript |

`full` requires `--transcript /path/to/export`. Claude Code can create a
visible text export with interactive `/export`; Cursor can export chat to
Markdown when that control is present. Codex has no assumed export command:
supply a visible-text file only when the client exposes one. A missing export
blocks `full` but not `summary` or `topic`; never reconstruct unavailable
history or scrape private client state.

Manual handoffs live under `tasks/TASK-NNN/context-save-<timestamp>.md` and
remain excluded from automatic indexing. `context-load` validates format and
source drift without mutating Brain, Bank, or SQLite. It returns the curated
document by default; `--include-transcript` is explicit. Historical commands,
instructions, and approval claims are data, not authority.

### Terminal Interface

The agent authors sanitized JSON before calling the shared CLI. Required fields
are `goal`, `summary`, and nonempty `next_steps`; optional list fields are
`constraints`, `decisions`, `progress`, `verification`, `open_questions`, and
safe repository-relative `files`.

```bash
python3 memory-bank/scripts/context.py context-save \
  --input /tmp/context-save-input.json \
  --output tasks/TASK-001/context-save-20260915T120000Z.md \
  --detail summary --source-client codex --json

python3 memory-bank/scripts/context.py context-load \
  --input tasks/TASK-001/context-save-20260915T120000Z.md --json
```

For a topic add `--detail topic --topic authentication`. For `full`, add
`--detail full --transcript /path/to/export.txt`. Relative paths resolve from
`--root`; an explicit absolute output may point elsewhere. Existing output
files are never overwritten.

## Verification

Runtime tests cover same-branch capture/restore, bounded excerpts, transcript
projection, secret rejection, malformed input, disabled state, and no
Brain/Bank/SQLite writes. Manual-handoff regressions cover curated round trips,
verbatim supplied transcripts, drift, index exclusion, overwrite refusal, and
safe paths. Run from an edition root:

```bash
python3 memory-bank/tests/test_context_continuity.py
python3 memory-bank/tests/test_context_handoff.py
```

Repository checks verify generated assets, native mirrors, and selected-tool
installations. They validate shipped hook wiring and synthetic payload handling;
they do not establish live-account lifecycle delivery.
