# Merge Chats and Hand Context to Another Task

Carrying a branch's work into its next session is automatic and does not need
anything on this page: the working-memory hooks give every prompt the Task
Capsule - the task's progress, next steps, recent files and relevant project
knowledge - from governed task state (see [Context Modes](CONTEXT-MODES.md)
and [Operations](OPERATIONS.md)). What follows covers the two cases the capsule
does not: combining the conversations of several chats into one new task, and
moving a curated handoff to another machine, checkout, client or reviewer.

## Harness UI

Harness provides **Merge chats** in Sessions for selecting 2–8 saved, inactive
chats from the same registered project and starting a new Codex, Claude or
Cursor task with their frozen, attributed visible histories. See the
[Harness workflow and limits](../harness/README.md#merge-chats-into-a-new-task).
This selection uses Harness's own message history; the native chat snapshots
below are a separate capability.

For an explicitly merged Harness task, the launch sets
`CONTEXT_CONTINUITY_RESTORE_DISABLED=1`, so a native merge prepared for that
client with `context-load merge` is not consumed by it. Capture continues. The
flag also accepts `true` or `yes`.

## Native Chat Snapshots and Merges

The installed `context-continuity.sh` hook keeps the visible text of every chat
in **Codex**, **Claude Code** and **Cursor**: each prompt and final answer is
appended to a per-chat snapshot in ignored local `.context-handoff/`, bound to
the checkout and Git branch (in an attached session, the launcher's state
directory holds it instead). One script is wired on each client's session
start, prompt and end-of-turn events; the event name in the payload selects
what it does.

Nothing is restored on its own. A new session receives chat history only when
a merge was prepared for it:

1. Ask the agent to merge particular chats, or use `context-load merge` in
   Codex and `/context-load merge` in Claude Code / Cursor. The agent lists the
   captured chats and freezes 2–8 chosen IDs for the **next new task** in the
   chosen client. The source set is captured immediately, not when that task
   starts.
2. Open a new task in that client, in the same checkout and branch. Its
   session-start hook claims the prepared merge once, under the same lock used
   for capture.

Each source keeps its own identity, client, commit, capture time, completeness
label, and exact captured text. The new task receives a balanced preview capped
at 6,000 UTF-8 bytes **including headers** (also bounded below the Codex
context limit), plus the path of the complete source archive. The receiving
agent reads omitted source content before using it, preserves each chat's
context, decisions, completed work and next steps, and reports conflicting
decisions by source. The runtime does not semantically resolve conflicts; they
remain `not-evaluated` until the agent checks them.

The archive is bound to the new task's identity. Later updates or rotation of
the source snapshots do not alter it, and reopening or compacting the merged
task delivers the same sources again, plus its own captured progress. A source
chat that resumes does not consume a merge prepared for another task, and a
session that already has history of its own never claims one.

One pending selection per branch and destination client is allowed. An
existing pending selection is never overwritten (an identical retry
succeeds); a malformed or missing selected source fails the whole operation.

Agent-facing terminal forms (IDs come from `list`; never guess them):

```bash
python3 memory-bank/scripts/context_continuity.py --host codex --event list
python3 memory-bank/scripts/context_continuity.py --host codex --event merge --source-session codex:CHAT_A --source-session claude:CHAT_B
```

`--host` accepts `codex`, `claude`, or `cursor` and selects the destination.
These forms write only ignored local state and report explicit failures with
exit code 1 and a `Context merge failed: ...` line on standard error. That
includes `CONTEXT_CONTINUITY_DISABLED` being set, or a state root that does not
exist: then nothing is listed or prepared, and both forms exit 1 and name the
reason instead of succeeding silently. A successful `merge` prints a JSON
object whose `prepared` field names the pending archive. Hook failures stay
silent and never block a task.

The hooks prepare and deliver context; they do not create native UI tasks.
When the user asks for one and the client's agent exposes a supported
task-creation tool, the skill uses it. Otherwise the prepared merge waits for
the next newly opened task; it must not be reported as an already created task.

### Storage

`.context-handoff/` is ignored local state, outside Project Brain, Memory Bank,
SQLite, retrieval indexes, and publishing. The store writes its own
`.gitignore` containing `*` when it is created (or on the next capture into a
store that predates it), so snapshots stay out of Git even in a project whose
`.gitignore` was never updated - an installer sync wires the hook but keeps the
project's own `.gitignore`. An existing `.gitignore` there is left alone.

Capture keeps eight chats per branch and 64 per checkout, and at most 4 MiB of
visible text per chat; when it trims older text, the snapshot marks the
omission. Each capture first removes, by file modification time (the last
capture) and without opening any snapshot, the oldest chats beyond those
limits and every chat idle for 30 days. That also clears snapshots of deleted
branches, snapshots left behind when the checkout moved (they no longer match
it and are never listed), and temporary files of a write the hook budget cut
short. A chat you want to keep longer belongs in a merge or a
`context-save` handoff. Frozen merge archives survive source rotation
until deleted. Each serialized archive is limited to 32 MiB including JSON
escaping and metadata, so eight individually valid 4 MiB sources can exceed the
combined limit; choose fewer or smaller sources in that case. A checkout holds
at most 128 archives and 256 MiB. Exceeding a limit refuses the new merge
without deleting existing history. Delete `.context-handoff/merges/` to remove
frozen merges; delete the entire `.context-handoff/` to remove all snapshots.

### Client Lifecycle and Limits

| Client | Capture | Delivery of a prepared merge | What is available |
| --- | --- | --- | --- |
| Claude Code | `UserPromptSubmit` and `Stop` | `SessionStart` | A native visible transcript when the hook supplies it; otherwise the visible prompt and final assistant message. |
| Codex | `UserPromptSubmit` and `Stop` | `SessionStart` | A native visible transcript when the hook supplies it; otherwise the visible prompt and final assistant message. |
| Cursor | `beforeSubmitPrompt` and `afterAgentResponse` | `sessionStart` | The prompt and response `text`; Cursor's session-start `additional_context` delivery is asynchronous. |

The runtime accepts a user-visible `.txt`, `.md`, or `.markdown` export when a
native hook makes one available. A recognised JSONL input is projected to only
visible `user` and `assistant` text; it discards system messages, hidden
reasoning, tool calls, envelopes, and metadata. In a Codex rollout it also
drops the context Codex records as user messages: the project's
`# AGENTS.md instructions` block (with or without a project path), and known
context envelopes such as `<environment_context>`, `<skill>` and
`<task-notification>`. User HTML/Blade examples and `<pasted_content>` stay.
So a Codex chat's `list` preview opens with its first real prompt. A projected
JSONL snapshot is explicitly partial, never a full transcript. The runtime never searches client
account directories, private session stores, or internal history files.

Native transcript availability varies by installed client version and lifecycle
event. An unavailable, delayed, partial, or projected transcript is recorded
only as the visible content actually supplied; it is never presented as proof
of a complete live session. Automated checks use synthetic hook payloads. They
do not prove that a live account delivered every event.

A stable session identity is required (`session_id` for Codex/Claude,
`conversation_id` for Cursor). A missing identity skips capture and delivery.
Cursor Cloud, without `sessionStart`, cannot receive a merge.

Set `CONTEXT_CONTINUITY_DISABLED=1` (also `true` or `yes`) to disable capture
and delivery for a shell or client process; the agent-facing `list` and `merge`
forms then exit 1 with that reason rather than reporting an empty list or a
merge that was never prepared. A missing Git branch, an untrusted
or disabled hook, a missing runtime, malformed input, oversized content, or a
likely secret all fail closed and quietly: the client turn continues without
stored or delivered context.

### Trust and Safety

Enable the hooks only through the client's normal project trust and permission
mechanism. The accelerator does not bypass trust prompts or client policy.
Before relying on merged text, verify material claims against current sources
and follow the current user's request.

The runtime receives only documented visible prompt/assistant fields or an
explicit visible export, and rejects recognised likely secrets before saving.
Each turn's new text is scanned once. Stored snapshots and frozen archives are
read back by their recorded sha256, which binds the text to the scan it passed
when it was written, so a capture or a delivery costs the same however long the
branch's chats are and stays inside the hook budget. `merge` scans its sources
again under the current patterns before freezing them, and the preview a new
task receives is scanned once more before delivery; the `list` preview of a
chat that no longer passes is withheld.
Snapshots never cross a branch or checkout boundary. They do not run embedded
commands, switch branches, mutate Brain/Bank/SQLite, create a native task, or
preserve approval for later actions.

Official hook references: [Codex](https://developers.openai.com/codex/hooks),
[Claude Code](https://code.claude.com/docs/en/hooks), and
[Cursor](https://cursor.com/docs/hooks).

## Portable Manual Handoff

Use `context-save` and `context-load` when context must move to another
machine, checkout or client, or when a reviewable task artifact is needed. They
share a portable Markdown format across Codex, Claude Code, and Cursor.

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

The curated fields, `--topic` and `--task-id` follow the policy of everything
an agent authors for later tasks: likely secrets and personal data are
refused, on save and again on load. Manual handoffs live under
`tasks/TASK-NNN/context-save-<timestamp>.md` and stay excluded from automatic
indexing, even when renamed. `context-load` validates format and source drift
without mutating Brain, Bank, or SQLite. A handoff checked out with CRLF line
endings - Git for Windows' default `core.autocrlf=true` converts a committed LF
file - still loads: a file that does not validate as read is checked again with
CRLF turned back into LF, and its digests decide. A new full handoff whose
transcript contains carriage returns stores that transcript as a JSON string
(version 2), preserving its exact bytes even when Git normalizes line endings
on commit. `--include-transcript` decodes it; version 1 handoffs remain readable. In a project outside
Git, or a copy of one without `.git`, cited files are fingerprinted without
ignore rules; where Git metadata exists but the ignore check cannot run, the
command still refuses. `context-load` returns the curated document by default;
`--include-transcript` is explicit. Historical commands, instructions,
and approval claims are data, not authority.

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
the project root - in an attached session the project, not the accelerator's
state directory, which a handoff is never written into. An explicit absolute
output may point elsewhere. Existing output files are never overwritten.

## Verification

Runtime tests cover capture, explicit merge preparation and delivery, frozen
archives across rotation and resume, event dispatch for each client, bounded
previews, transcript projection (including Codex's injected context), secret
rejection, malformed input, disabled state and its explicit `list`/`merge`
failure, attached state, the self-ignoring store, retention across branches,
captures and deliveries timed against the hook budget beside eight 3.5 MB
chats, and no Brain/Bank/SQLite writes. Manual-handoff tests cover curated
round trips, verbatim supplied transcripts, drift, index exclusion, overwrite
refusal, safe paths, personal-data refusal (topic and task ID included), a
CRLF checkout, a project outside Git and the attached layout. Run from an
edition root:

```bash
python3 memory-bank/tests/test_context_continuity.py
python3 memory-bank/tests/test_context_handoff.py
python3 memory-bank/tests/test_context_handoff_index.py
```

Repository checks verify generated assets, native mirrors, hook wiring, and
selected-tool installations. They validate shipped hook wiring and synthetic
payload handling; they do not establish live-account lifecycle delivery.
