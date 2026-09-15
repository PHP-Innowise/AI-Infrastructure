# Continue Context Across Tasks

## Automatic Continuity

The installed accelerator automatically carries a compact, same-branch excerpt
between tasks in **Codex**, **Claude Code**, and **Cursor** when that client's
project hooks are enabled and trusted. It is the normal way to continue after a
new task, restart, or context compaction; no command or agent delegation is
needed.

At an available turn-end event, the hook passes only a documented visible-text
field to `memory-bank/scripts/context_continuity.py`. The runtime writes one
private snapshot under `.context-handoff/`, bound to the repository and current
Git branch. At the next supported session-start event it returns the most recent
same-branch snapshot as historical context. The visible excerpt is capped at
6,000 characters; the native hook envelope adds a short provenance header.
the current policy, code, specifications, and tests remain authoritative.

`.context-handoff/` is ignored by Git and is deliberately outside Project
Brain, Memory Bank, SQLite, automatic retrieval, task artifacts, and shared
chat publishing. It is local, ephemeral convenience state, not a work record
or an authorization transfer. Delete the directory to remove it.

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
Brain/Bank/SQLite, create a task, or preserve approval for later actions.

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
