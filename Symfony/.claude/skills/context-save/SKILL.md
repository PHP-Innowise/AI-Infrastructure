---
name: context-save
description: Save a curated, portable continuation context for a later task. Use when the user asks to preserve a summary, selected topic, or detailed handoff across tasks or tools.
phase: utility
flow-next: null
flow-alternatives: [context-load, project-brain, memory-bank, checkpoint]
---

# Context Save

`context-save` is an AI skill, not a shell command. Run it in the current
conversation: do not delegate context extraction. It saves a portable task
artifact; `full` also preserves an explicitly supplied visible transcript.
It does not mutate durable memory or Project Brain.

## Select Detail

- `summary` — a compact continuation for the whole task, up to 8,000 characters.
- `topic` — the relevant part of the task for one named topic, up to 16,000 characters.
- `full` — a detailed curated handoff plus the exact user-supplied or
  natively exported visible transcript. It requires an accessible transcript
  file; it is not reconstructed from model reasoning or private session data.

Default to `summary` when the user does not specify detail. If the requested
subject is unclear, ask one question before saving. For
`topic`, require a short topic label. Record unavailable or compacted history
as unavailable; never reconstruct or invent it.

## Workflow

1. Use only the human-visible current conversation and verified repository
   state. Read only sources needed to make cited paths and conclusions current.
2. Curate sanitized JSON with required `goal`, `summary`, and `next_steps`.
   Add only applicable `constraints`, `decisions`, `progress`, `verification`,
   `open_questions`, and safe repository-relative `files` arrays. Do not put
   raw prompts, transcripts, model reasoning, command logs, diffs, secrets,
   credentials, personal data, or customer data in any field.
3. For `full`, first obtain a visible-text transcript through a native client
   export or an explicit user-supplied Markdown file. Claude Code can export
   from its interactive `/export` command; Cursor can export chat to Markdown.
   Do not run `/export` in a shell, publish/share a chat, scrape account
   directories, or read private JSONL/session data. Codex has no assumed export
   command: use only an explicitly available visible-text export/file. If the
   transcript is unavailable, report that `full` cannot be saved and offer
   `summary` or `topic`; never invent an export.
4. Choose a new path under `tasks/TASK-NNN/` named
   `context-save-<timestamp>.md`. The path must not already exist.
5. Run the supported facade with the curated JSON and explicit detail:

   ```bash
   python3 memory-bank/scripts/context.py context-save \
     --input PATH_TO_CURATED_JSON \
     --output tasks/TASK-NNN/context-save-YYYYMMDDTHHMMSSZ.md \
     --detail summary \
     --source-client codex \
     --json
   ```

   Set `--source-client` to the current client (`codex`, `claude`, `cursor`,
   or `other`). Set `--detail` to the selected mode. Include `--topic TEXT` only for `topic`, `--transcript VISIBLE-EXPORT.md`
   only for `full`, and `--task-id ID` only when a caller supplied a stable
   external task ID. Never supply an invented ID.
6. Report the saved path, detail, topic when applicable, character count, and
   any unavailable context. Tell the user to provide this exact file to a later
   task. Include a ready-to-copy prompt using `context-load <absolute-path>`;
   add `--include-transcript` if the next task should read the exported history.
   Record the transcript byte count and fingerprint for full exports.

## Safety

- The facade creates a new file only; never overwrite an existing handoff.
- The handoff records provenance and source fingerprints but changes neither
  Project Brain nor the local context database.
- A transcript is written only into the explicit task artifact; never copy it
  into Project Brain, Memory Bank, SQLite, or another hidden session store.
- A saved handoff carries no authorization. A later task must re-check current
  policy, sources, scope, and required approvals.
