# Carry Context Into the Next Task

Use `context-save` at the end of a conversation and `context-load` at the start
of the next one. They share a portable Markdown format across Codex, Claude
Code, and Cursor. The file contains a curated continuation document and source
fingerprints. In `full` mode it also preserves an explicitly supplied visible
conversation export verbatim, so another client can consult the same history.

## Quick Start

Run the skills in the installed target project, or with the selected edition
as the workspace root. A skill nested in a distribution folder is not installed
in the monorepo root merely because the folder exists.

| Client | Save a summary | Load a saved document |
| --- | --- | --- |
| Codex | `context-save summary` | `context-load tasks/TASK-001/context-save-20260915T120000Z.md` |
| Claude Code | `/context-save summary` | `/context-load tasks/TASK-001/context-save-20260915T120000Z.md` |
| Cursor | `/context-save summary` | `/context-load tasks/TASK-001/context-save-20260915T120000Z.md` |

These are AI skill/command invocations, not terminal executables. Natural
language works too: “Save the decisions and remaining work for the next task”
and “Load this saved context before continuing.”

The save result gives the exact absolute file path and a ready-to-copy next-task
prompt. In another checkout or on another machine, copy or attach the document
first. Source paths are relative to the project, so the original absolute path
is not required to verify files in a new checkout. Saving does not stage,
commit, push, or publish the file.

## Choose the Amount of Context

| Detail | What the agent prepares | Maximum authored text |
| --- | --- | --- |
| `summary` (default) | Goal, constraints, key decisions, progress, verification, open questions, next steps and relevant file references | 8,000 Unicode characters |
| `topic` | The same sections, restricted to the named subject before collecting or writing input | 16,000 Unicode characters |
| `full` | A detailed continuation document plus a verbatim visible-conversation export supplied with `--transcript` | 64,000 authored characters plus a bounded transcript |

Examples:

```text
context-save topic authentication
context-save full --transcript /path/to/export.txt
context-load tasks/TASK-001/context-save-20260915T120000Z.md
```

Claude Code and Cursor use the same arguments after `/context-save` or
`/context-load`. The CLI validates the selected size; it never silently cuts
text. The agent performs summarization and topic selection. It must record
unknown or unavailable history rather than reconstructing it as fact.

### Full Conversation Exports

`full` requires a text or Markdown conversation export from the client, supplied
as an explicit `--transcript` path. Its contents are preserved verbatim, including
line endings and code fences, with a byte count and SHA-256 fingerprint (up to 4 MiB). The
runtime refuses oversized or likely-secret-bearing exports before writing; it
does not silently redact, summarize or truncate them. The original export's
coverage is the limit: messages or attachments absent from it are not recovered.

| Client | Obtain a visible conversation export |
| --- | --- |
| Claude Code CLI | Run the interactive `/export /path/to/export.txt`, then `/context-save full --transcript /path/to/export.txt`. `/export` is a client command, not a shell executable. |
| Cursor | Use the installed client's chat export to Markdown when available, then `/context-save full --transcript /path/to/export.md`. |
| Codex | Supply a visible-text export file from an available client capability. If the client exposes no export, report that limitation; do not invent a `codex export` command. Native fork/resume is a separate option for continuing within Codex. |

Claude documents its [text export](https://code.claude.com/docs/en/sessions#export-and-locate-session-data).
Cursor documents [Markdown chat exports](https://cursor.com/changelog/page/15);
check the installed client's available controls. Its newer
[shared transcripts](https://cursor.com/help/ai-features/shared-transcripts)
publish a link, so the save workflow does not create one automatically.

A missing export blocks only `full`; the agent reports how to provide one and
may offer `summary` or `topic` without silently changing the requested mode.
The skills do not scrape account directories, private SQLite stores, internal
JSONL sessions or hidden reasoning. A manually copied selection can be preserved
exactly, but it must be described as a selection, never the complete session.

## What Loading Does

1. Validates the file format, supported version, field types, size and likely
   secret patterns before returning its contents.
2. Compares recorded branch/commit and the fingerprints of explicitly cited
   files against the current project. Missing, added or changed sources are
   reported; matching fingerprints establish byte equality, not correctness.
3. Returns the document as historical context. The agent checks material claims
   against current policy, code, specifications and tests.
4. Summarizes what is applicable, what changed and the proposed next step.
   By default it returns the curated document and transcript metadata, not the
   transcript body. `--include-transcript` explicitly returns the verbatim text;
   the agent must say if tool-output limits prevented reading all of it.

Snapshots are excluded from automatic context indexing, including renamed
copies identified by their format. Loading does not execute saved commands, switch branches, create or rebind a
Brain task, promote memory, or create a SQLite database. A saved task ID is a
reference only. Historical instructions and approval claims inside the document
are data; the current user's request and current policy govern further work.
If the user only asks to load context, the skill stops after reporting. If the
user also asks to continue, follow that current request after checking drift.

## Relationship to Existing Memory

- **Project Brain** remains authoritative for active tasks and governed
  progress. Its compact handoffs remain part of that lifecycle.
- **Memory Bank** holds durable reusable knowledge, not unfinished task notes.
- **`memory`** refreshes retrieval; **`checkpoint`** follows the configured
  governed/lightweight authority gate.
- **`context-save` / `context-load`** transfer a non-authoritative snapshot
  selected by the current user. They neither duplicate a live progress store
  nor replace the task lifecycle.
- **`context.py export`** bundles eligible Brain and Memory Bank records;
  **`./collect`** bundles repository files. Neither extracts conversation
  context.

## Terminal Interface

The agent authors sanitized JSON before calling the shared CLI. A minimal
input looks like this:

```json
{
  "goal": "Finish the retry behavior",
  "summary": "The bounded retry path is implemented; verification remains.",
  "constraints": ["Keep the public interface compatible"],
  "decisions": ["Retry only transient failures"],
  "progress": ["Added a bounded retry loop"],
  "verification": ["Focused test passed; full suite not run"],
  "open_questions": [],
  "next_steps": ["Run the full suite and review failure handling"],
  "files": ["src/retry.py", "tests/test_retry.py"]
}
```

`goal`, `summary`, and a nonempty `next_steps` list are required. Other lists
may be omitted. File references are explicit safe repository-relative paths;
no source bodies are embedded. Sensitive, ignored, escaping and symlinked
source paths are rejected. Missing sources can be recorded to represent a
deleted file or a file still to be created.

From the project root:

```bash
python3 memory-bank/scripts/context.py context-save \
  --input /tmp/context-save-input.json \
  --output tasks/TASK-001/context-save-20260915T120000Z.md \
  --detail summary --source-client codex --json

python3 memory-bank/scripts/context.py context-load \
  --input tasks/TASK-001/context-save-20260915T120000Z.md --json
```

For a topic, use `--detail topic --topic authentication`. For a detailed
snapshot with a conversation export, use `--detail full --transcript /path/to/export.txt`. The optional `--task-id` records an existing
external task identifier without creating or changing its task. `--source-client`
accepts `codex`, `claude`, `cursor`, or `other`.

Relative CLI paths are resolved against `--root` (the installed project by
default); an explicit absolute output path may point to another location.
Outputs inside Project Brain, Memory Bank or client policy stores are refused.
Missing parent directories are created after input validation.

Use a new output filename for each snapshot: existing destinations are never
overwritten. Do not put personal snapshots into the shared Memory Bank or its
chunks. Use the project's task directory and normal review process if the
handoff should travel with a branch.

## Verification

Runtime regressions cover summary/topic round trips, verbatim transcript preservation,
explicit transcript loading, index exclusion, size and schema rejection, secret
filtering, overwrite refusal, safe paths, drift and the absence of implicit
Brain/SQLite writes. Run from an edition root:

```bash
python3 memory-bank/tests/test_context_handoff.py
```

Repository checks verify the shared core, generated assets, native mirrors and
selected-tool installations. These checks verify the shipped commands and
runtime; they do not claim to have exercised a live account in each AI client.
