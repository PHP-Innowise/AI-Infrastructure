# Automatic Working-Memory Checkpoint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one argument-free AI command named `checkpoint` that captures all current Git-visible changes as sanitized repository-local Working Memory.

**Architecture:** A tool-native command delegates to a mirrored repository skill. The active AI agent derives the task ID from the current branch, inspects safe Git changes, writes the semantic summary, and reuses the existing `context.py get/start/update` lifecycle; SQLite schema and Context Engine Python code remain unchanged.

**Tech Stack:** Markdown command/skill contracts, Git porcelain v1, existing Python 3.9+ Context Engine, SQLite FTS5, Python stdlib `unittest`.

## Global Constraints

- The feature applies to the Laravel, PHP Core, and Symfony accelerators and keeps their integrations synchronized.
- The user invokes `checkpoint` without arguments.
- Claude and Cursor expose `/checkpoint`; Codex exposes the equivalent repository skill by the same name.
- The current Git branch is the task ID; detached HEAD is rejected.
- All current staged, unstaged, renamed, deleted, copied, and untracked non-ignored paths are included.
- Pre-existing or unrelated dirty files are included deliberately.
- Binary and sensitive file contents are not read.
- Only normalized paths and a sanitized semantic summary are stored; raw diffs and file contents are not stored.
- A missing Working Memory task is created automatically; an existing task is updated.
- No-change execution is a no-op.
- `checkpoint` never stages, commits, discards, completes, or creates an episode.
- Do not add a model call, service, dependency, Git hook, SQLite migration, or new Context Engine storage path.
- `Laravel/memory-bank/scripts/context.py`, `PHP Core/memory-bank/scripts/context.py`, and `Symfony/memory-bank/scripts/context.py` must remain byte-identical and unchanged.
- Existing secret-pattern validation remains the final storage boundary.
- Existing unrelated untracked `.langgraph_api/` and `ai_infrastructure_langgraph.egg-info/` stay untouched and unstaged.

---

### Task 1: Add the checkpoint command and repository skill

**Files:**

- Create: `Symfony/memory-bank/tests/test_checkpoint.py`
- Create: `Laravel/memory-bank/tests/test_checkpoint.py`
- Create: `PHP Core/memory-bank/tests/test_checkpoint.py`
- Create: `Symfony/.claude/skills/checkpoint/SKILL.md`
- Create: `Symfony/.cursor/skills/checkpoint/SKILL.md`
- Create: `Symfony/.agents/skills/checkpoint/SKILL.md`
- Create: `Laravel/.claude/skills/checkpoint/SKILL.md`
- Create: `Laravel/.cursor/skills/checkpoint/SKILL.md`
- Create: `Laravel/.agents/skills/checkpoint/SKILL.md`
- Create: `PHP Core/.claude/skills/checkpoint/SKILL.md`
- Create: `PHP Core/.cursor/skills/checkpoint/SKILL.md`
- Create: `PHP Core/.agents/skills/checkpoint/SKILL.md`
- Create: `Symfony/.claude/commands/checkpoint.md`
- Create: `Symfony/.cursor/commands/checkpoint.md`
- Create: `Laravel/.claude/commands/checkpoint.md`
- Create: `Laravel/.cursor/commands/checkpoint.md`
- Create: `PHP Core/.claude/commands/checkpoint.md`
- Create: `PHP Core/.cursor/commands/checkpoint.md`

**Interfaces:**

- Consumes: existing `memory-bank/scripts/context.py get|start|update` CLI and repository Git state.
- Produces: argument-free Claude/Cursor command wrappers and a `checkpoint` skill with equivalent behaviour in all three accelerators.
- Produces: `CheckpointIntegrationTest`, a deterministic static contract test for tool discovery and safety requirements.

- [ ] **Step 1: Write the failing checkpoint integration test in Symfony**

Create `Symfony/memory-bank/tests/test_checkpoint.py`:

```python
#!/usr/bin/env python3
"""Contract tests for the argument-free checkpoint agent command."""

from __future__ import annotations

import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SKILL_PATHS = (
    ".claude/skills/checkpoint/SKILL.md",
    ".cursor/skills/checkpoint/SKILL.md",
    ".agents/skills/checkpoint/SKILL.md",
)
COMMAND_PATHS = (
    (".claude/commands/checkpoint.md", ".claude/skills/checkpoint/SKILL.md"),
    (".cursor/commands/checkpoint.md", ".cursor/skills/checkpoint/SKILL.md"),
)


class CheckpointIntegrationTest(unittest.TestCase):
    def test_tool_skills_are_byte_identical(self) -> None:
        contents = [
            REPOSITORY_ROOT.joinpath(path).read_bytes()
            for path in SKILL_PATHS
        ]

        self.assertEqual(contents[0], contents[1])
        self.assertEqual(contents[0], contents[2])

    def test_skill_defines_the_checkpoint_contract(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(
            encoding="utf-8"
        )
        required = (
            "git rev-parse --show-toplevel",
            "git symbolic-ref --quiet --short HEAD",
            "git status --porcelain=v1 -z --untracked-files=all",
            "staged, unstaged, and untracked",
            "renamed, copied, and deleted",
            "memory-bank/scripts/context.py get",
            "memory-bank/scripts/context.py start",
            "memory-bank/scripts/context.py update",
            "Checkpoint work on branch",
            "MUST NOT read binary",
            "MUST NOT read `.env`",
            "MUST NOT store raw diffs",
            "MUST NOT run `complete`",
            "no current Git changes",
            "detached HEAD",
        )

        for text in required:
            with self.subTest(text=text):
                self.assertIn(text, skill)

    def test_command_wrappers_accept_no_user_arguments(self) -> None:
        forbidden = (
            "--task-id",
            "--summary",
            "--complete",
            "--outcome",
            "--verification",
        )

        for command_path, skill_path in COMMAND_PATHS:
            with self.subTest(command=command_path):
                command = REPOSITORY_ROOT.joinpath(command_path).read_text(
                    encoding="utf-8"
                )
                self.assertIn(skill_path, command)
                self.assertIn("accepts no arguments", command)
                for option in forbidden:
                    self.assertNotIn(option, command)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Mirror the failing test into Laravel and PHP Core**

Create byte-identical copies at:

```text
Laravel/memory-bank/tests/test_checkpoint.py
PHP Core/memory-bank/tests/test_checkpoint.py
```

Use `apply_patch` for each file. Confirm the three copies before running them:

```bash
cmp Symfony/memory-bank/tests/test_checkpoint.py Laravel/memory-bank/tests/test_checkpoint.py
cmp Symfony/memory-bank/tests/test_checkpoint.py "PHP Core/memory-bank/tests/test_checkpoint.py"
```

Expected: both commands exit `0`.

- [ ] **Step 3: Run the new test in all three accelerators and verify it fails**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_checkpoint.py' -v
```

Expected: each run fails with `FileNotFoundError` because checkpoint skills and commands do not exist yet.

- [ ] **Step 4: Create the canonical checkpoint skill**

Create the following byte-identical content in:

```text
Symfony/.claude/skills/checkpoint/SKILL.md
Symfony/.cursor/skills/checkpoint/SKILL.md
Symfony/.agents/skills/checkpoint/SKILL.md
Laravel/.claude/skills/checkpoint/SKILL.md
Laravel/.cursor/skills/checkpoint/SKILL.md
Laravel/.agents/skills/checkpoint/SKILL.md
PHP Core/.claude/skills/checkpoint/SKILL.md
PHP Core/.cursor/skills/checkpoint/SKILL.md
PHP Core/.agents/skills/checkpoint/SKILL.md
```

```markdown
---
name: checkpoint
description: Capture all current Git-visible changes as sanitized repository-local Working Memory. Use when the user invokes checkpoint or asks to save current AI work without supplying lifecycle arguments.
phase: utility
flow-next: null
flow-alternatives: [memory-bank, verify]
---

# Working-Memory Checkpoint

Capture one progress snapshot and stop. This skill accepts no arguments.

## Workflow

1. Resolve the repository root with `git rev-parse --show-toplevel`. Stop without
   touching Context Engine state when this fails.
2. Resolve the task ID with `git symbolic-ref --quiet --short HEAD`. Stop with
   an actionable `detached HEAD` error when it returns no branch. Do not invent
   another ID.
3. From the repository root, read
   `git status --porcelain=v1 -z --untracked-files=all`. Parse the NUL-delimited
   output and collect every current staged, unstaged, and untracked non-ignored
   path, including renamed, copied, and deleted paths.
4. If there are no current Git changes, report a no-op and stop without
   creating or updating Working Memory.
5. Inspect the staged and unstaged diffs plus safe untracked text files only as
   needed to understand the change. Pre-existing or unrelated dirty files are
   part of the snapshot.
6. Write a concise semantic progress summary. Sanitize it before storage and
   never copy raw diff text, command output, prompts, responses, or secrets.
7. Run `python3 memory-bank/scripts/context.py get --task-id <branch> --json`.
   If the task does not exist, run
   `python3 memory-bank/scripts/context.py start --task-id <branch> --goal
   "Checkpoint work on branch <branch>"` with one `--file` value per normalized
   changed path.
8. Run `python3 memory-bank/scripts/context.py update --task-id <branch>
   --progress <summary>` and include one `--file` value per normalized changed
   path. Pass generated values as argv data; never interpolate Git content into
   executable shell syntax.
9. Report the task ID, current changed-file count, whether the task was created
   or updated, and the saved summary. Then stop.

## Safety

- MUST NOT read binary file contents.
- MUST NOT read `.env` files, private keys, credential files, or other paths
  prohibited by repository policy.
- MUST NOT store raw diffs or file contents.
- MUST NOT run `complete`, create an episode, stage, commit, discard, or modify
  repository files.
- MUST keep Git-ignored files excluded.
- MUST allow existing Context Engine task-ID and secret validation to fail
  safely without replacing valid Working Memory.
```

- [ ] **Step 5: Create the Claude and Cursor command wrappers**

For each accelerator, create `.claude/commands/checkpoint.md`:

```markdown
---
phase: utility
flow-next: null
flow-alternatives: [memory-bank, verify]
---

# Checkpoint

This command accepts no arguments. If `$ARGUMENTS` is not empty, stop and ask
the user to invoke `/checkpoint` without arguments.

Read `.claude/skills/checkpoint/SKILL.md`, execute exactly one Working-Memory
checkpoint in the current agent, and stop.
```

For each accelerator, create `.cursor/commands/checkpoint.md`:

```markdown
---
name: checkpoint
description: "Capture all current Git-visible changes as local Working Memory."
---

# Checkpoint

This command accepts no arguments. If `$ARGUMENTS` is not empty, stop and ask
the user to invoke `/checkpoint` without arguments.

Read `.cursor/skills/checkpoint/SKILL.md`, execute exactly one Working-Memory
checkpoint in the current agent, and stop.
```

- [ ] **Step 6: Run the checkpoint integration tests**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_checkpoint.py' -v
```

Expected: `3 tests` pass in each accelerator.

- [ ] **Step 7: Verify all new mirrors**

Run:

```bash
sha256sum \
  Symfony/.claude/skills/checkpoint/SKILL.md \
  Symfony/.cursor/skills/checkpoint/SKILL.md \
  Symfony/.agents/skills/checkpoint/SKILL.md \
  Laravel/.claude/skills/checkpoint/SKILL.md \
  Laravel/.cursor/skills/checkpoint/SKILL.md \
  Laravel/.agents/skills/checkpoint/SKILL.md \
  "PHP Core/.claude/skills/checkpoint/SKILL.md" \
  "PHP Core/.cursor/skills/checkpoint/SKILL.md" \
  "PHP Core/.agents/skills/checkpoint/SKILL.md"
sha256sum \
  Symfony/memory-bank/tests/test_checkpoint.py \
  Laravel/memory-bank/tests/test_checkpoint.py \
  "PHP Core/memory-bank/tests/test_checkpoint.py"
```

Expected: one hash for all nine skills and one hash for all three tests.

- [ ] **Step 8: Commit the checkpoint integration**

Stage only the new checkpoint skills, command wrappers, and tests:

```bash
git add -- \
  Symfony/.claude/skills/checkpoint/SKILL.md \
  Symfony/.cursor/skills/checkpoint/SKILL.md \
  Symfony/.agents/skills/checkpoint/SKILL.md \
  Symfony/.claude/commands/checkpoint.md \
  Symfony/.cursor/commands/checkpoint.md \
  Symfony/memory-bank/tests/test_checkpoint.py \
  Laravel/.claude/skills/checkpoint/SKILL.md \
  Laravel/.cursor/skills/checkpoint/SKILL.md \
  Laravel/.agents/skills/checkpoint/SKILL.md \
  Laravel/.claude/commands/checkpoint.md \
  Laravel/.cursor/commands/checkpoint.md \
  Laravel/memory-bank/tests/test_checkpoint.py \
  "PHP Core/.claude/skills/checkpoint/SKILL.md" \
  "PHP Core/.cursor/skills/checkpoint/SKILL.md" \
  "PHP Core/.agents/skills/checkpoint/SKILL.md" \
  "PHP Core/.claude/commands/checkpoint.md" \
  "PHP Core/.cursor/commands/checkpoint.md" \
  "PHP Core/memory-bank/tests/test_checkpoint.py"
git diff --cached --check
git commit -m "feat: add automatic working-memory checkpoint"
```

Expected: one commit containing exactly 18 new files.

---

### Task 2: Align policy and user documentation with checkpoint

**Files:**

- Modify: `Symfony/memory-bank/tests/test_checkpoint.py`
- Modify: `Laravel/memory-bank/tests/test_checkpoint.py`
- Modify: `PHP Core/memory-bank/tests/test_checkpoint.py`
- Modify: `Symfony/AGENTS.md`
- Modify: `Laravel/AGENTS.md`
- Modify: `PHP Core/AGENTS.md`
- Modify: `Symfony/memory-bank/README.md`
- Modify: `Laravel/memory-bank/README.md`
- Modify: `PHP Core/memory-bank/README.md`
- Modify: `Symfony/README.md`
- Modify: `Laravel/README.md`
- Modify: `PHP Core/README.md`
- Modify: `README.md`

**Interfaces:**

- Consumes: the `checkpoint` skill and command contract from Task 1.
- Produces: policy that permits the automatic branch-derived flow without removing the existing explicit lifecycle.
- Produces: documentation that makes `checkpoint` the shortest progress-capture path and keeps `context`/`complete` explicit.

- [ ] **Step 1: Add failing policy and documentation assertions**

Add these methods to `CheckpointIntegrationTest` in
`Symfony/memory-bank/tests/test_checkpoint.py`, then apply the same change to
the Laravel and PHP Core copies:

```python
    def test_policy_allows_argument_free_checkpoint(self) -> None:
        policy = REPOSITORY_ROOT.joinpath("AGENTS.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("checkpoint", policy)
        self.assertIn("current Git branch", policy)
        self.assertIn("automatically creates or updates", policy)

    def test_readmes_document_checkpoint_without_hiding_completion(self) -> None:
        memory_readme = REPOSITORY_ROOT.joinpath(
            "memory-bank/README.md"
        ).read_text(encoding="utf-8")
        accelerator_readme = REPOSITORY_ROOT.joinpath("README.md").read_text(
            encoding="utf-8"
        )

        for document in (memory_readme, accelerator_readme):
            self.assertIn("checkpoint", document)
            self.assertIn("complete", document)
```

- [ ] **Step 2: Run the focused tests and verify the new assertions fail**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_checkpoint.py' -v
```

Expected: the two new tests fail because current policy and READMEs still
describe only the explicit `start → update` flow.

- [ ] **Step 3: Update the three AGENTS.md policies**

In the `Agent Behavior` section of each accelerator's `AGENTS.md`, replace the
two explicit-lifecycle bullets with:

```markdown
- MUST use the argument-free `checkpoint` skill when the user asks to capture
  current progress: derive the task ID from the current Git branch, include all
  current Git-visible changes, and save a sanitized summary; the skill
  automatically creates or updates Working Memory.
- MUST use a caller-supplied task ID only for the manual
  `start → update → context → complete` lifecycle. `checkpoint` MUST NOT
  complete the task or create an episode.
```

In the `Memory Bank` section, add:

```markdown
- `checkpoint` is the preferred argument-free progress capture. It stores a
  sanitized summary and changed paths in Working Memory; explicit `complete`
  remains required after verification.
```

Preserve all framework-specific policy outside these exact bullets.

- [ ] **Step 4: Update the three memory-bank READMEs**

In each `memory-bank/README.md`, add `checkpoint` as the preferred routine
progress capture immediately before the manual lifecycle example:

```markdown
For routine progress capture, invoke the AI command `checkpoint` with no
arguments. The active agent derives the task ID from the current Git branch,
summarizes all current Git-visible changes, and automatically creates or updates
Working Memory. It does not complete the task.
```

Keep the existing CLI example but label `start` and `update` as the manual flow.
Keep `context` and `complete` explicit because retrieval needs a task/query and
completion needs a verified outcome.

- [ ] **Step 5: Update the three accelerator READMEs**

In each framework README:

1. Add `checkpoint` to the skill/command table:

```markdown
| `checkpoint` | Capture all current Git-visible changes as local Working Memory without arguments |
```

2. In the Memory Bank section, add:

```markdown
Use `checkpoint` in Codex or `/checkpoint` in Claude/Cursor to capture current
progress without lifecycle arguments. The branch name becomes the task ID; the
agent stores a sanitized summary and changed paths. Use explicit `complete`
only after verification.
```

Retain the existing framework-specific wording and explicit durable-memory
guidance.

- [ ] **Step 6: Update the root Russian README**

In the root `README.md` Working Memory and command examples, describe:

```markdown
Для обычной контрольной точки достаточно вызвать `checkpoint` без аргументов
(`checkpoint` в Codex, `/checkpoint` в Claude и Cursor). Агент возьмёт task-id
из имени текущей Git-ветки, проанализирует все staged, unstaged и untracked
изменения и сохранит безопасное резюме со списком файлов в Working Memory.
Завершение задачи остаётся отдельным явным действием через `complete`.
```

Keep the manual `start`, `update`, `context`, and `complete` example as the
advanced explicit lifecycle. Do not start the separate bilingual README plan in
this task.

- [ ] **Step 7: Run focused tests and documentation validation**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_checkpoint.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_checkpoint.py' -v
python3 Symfony/memory-bank/scripts/validate.py
python3 Laravel/memory-bank/scripts/validate.py
python3 "PHP Core/memory-bank/scripts/validate.py"
git diff --check
```

Expected: `5 tests` pass in each accelerator, all three validators pass, and
`git diff --check` prints nothing.

- [ ] **Step 8: Verify test mirror parity**

Run:

```bash
cmp Symfony/memory-bank/tests/test_checkpoint.py Laravel/memory-bank/tests/test_checkpoint.py
cmp Symfony/memory-bank/tests/test_checkpoint.py "PHP Core/memory-bank/tests/test_checkpoint.py"
```

Expected: both commands exit `0`.

- [ ] **Step 9: Commit policy and documentation**

```bash
git add -- \
  README.md \
  Symfony/AGENTS.md \
  Symfony/README.md \
  Symfony/memory-bank/README.md \
  Symfony/memory-bank/tests/test_checkpoint.py \
  Laravel/AGENTS.md \
  Laravel/README.md \
  Laravel/memory-bank/README.md \
  Laravel/memory-bank/tests/test_checkpoint.py \
  "PHP Core/AGENTS.md" \
  "PHP Core/README.md" \
  "PHP Core/memory-bank/README.md" \
  "PHP Core/memory-bank/tests/test_checkpoint.py"
git diff --cached --check
git commit -m "docs: make checkpoint the simple working-memory flow"
```

Expected: one commit containing only the listed policy, README, and focused
test updates.

---

### Task 3: Run full mirror and real-project verification

**Files:**

- Verify only: all files created or modified in Tasks 1 and 2.
- External fixture: `/home/aliaksei/Desktop/bauherrenmappe` (read-only source).
- Temporary artifacts: a directory created with `mktemp -d`, removed after the test.

**Interfaces:**

- Consumes: completed checkpoint integration and existing Context Engine.
- Produces: concrete full-suite, mirror-parity, and Bauherrenmappe black-box evidence.
- Produces no repository files or commit.

- [ ] **Step 1: Run every memory-bank test in all three accelerators**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_*.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_*.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_*.py' -v
```

Expected: every test passes in all three runs, including the five checkpoint
contract tests.

- [ ] **Step 2: Run all memory-bank validators**

Run:

```bash
python3 Symfony/memory-bank/scripts/validate.py
python3 Laravel/memory-bank/scripts/validate.py
python3 "PHP Core/memory-bank/scripts/validate.py"
```

Expected: all three validators exit `0`.

- [ ] **Step 3: Verify unchanged Context Engine parity and new mirror parity**

Run:

```bash
cmp Symfony/memory-bank/scripts/context.py Laravel/memory-bank/scripts/context.py
cmp Symfony/memory-bank/scripts/context.py "PHP Core/memory-bank/scripts/context.py"
cmp Symfony/memory-bank/tests/test_context.py Laravel/memory-bank/tests/test_context.py
cmp Symfony/memory-bank/tests/test_context.py "PHP Core/memory-bank/tests/test_context.py"
cmp Symfony/memory-bank/tests/test_checkpoint.py Laravel/memory-bank/tests/test_checkpoint.py
cmp Symfony/memory-bank/tests/test_checkpoint.py "PHP Core/memory-bank/tests/test_checkpoint.py"
sha256sum \
  Symfony/.claude/skills/checkpoint/SKILL.md \
  Symfony/.cursor/skills/checkpoint/SKILL.md \
  Symfony/.agents/skills/checkpoint/SKILL.md \
  Laravel/.claude/skills/checkpoint/SKILL.md \
  Laravel/.cursor/skills/checkpoint/SKILL.md \
  Laravel/.agents/skills/checkpoint/SKILL.md \
  "PHP Core/.claude/skills/checkpoint/SKILL.md" \
  "PHP Core/.cursor/skills/checkpoint/SKILL.md" \
  "PHP Core/.agents/skills/checkpoint/SKILL.md"
```

Expected: every `cmp` exits `0` and all nine skill hashes are identical.

- [ ] **Step 4: Capture Bauherrenmappe's original state**

Run:

```bash
git -C /home/aliaksei/Desktop/bauherrenmappe status --porcelain=v1 -z > /tmp/bauherrenmappe-status-before
git -C /home/aliaksei/Desktop/bauherrenmappe rev-parse HEAD > /tmp/bauherrenmappe-head-before
git -C /home/aliaksei/Desktop/bauherrenmappe write-tree > /tmp/bauherrenmappe-index-before
```

If `/home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db` exists,
record its checksum without opening it:

```bash
sha256sum /home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db > /tmp/bauherrenmappe-context-before
```

Expected: all applicable commands exit `0`.

- [ ] **Step 5: Create a disposable clone of the real Bauherrenmappe project**

Run:

```bash
CHECKPOINT_FIXTURE=$(mktemp -d)
git clone --shared --no-hardlinks \
  /home/aliaksei/Desktop/bauherrenmappe \
  "$CHECKPOINT_FIXTURE/bauherrenmappe"
git -C "$CHECKPOINT_FIXTURE/bauherrenmappe" switch -c checkpoint-real-project
```

Expected: the clone succeeds and the fixture branch is
`checkpoint-real-project`.

- [ ] **Step 6: Create representative Git changes only in the disposable clone**

Use `apply_patch` against the resolved temporary clone path to add:

```text
checkpoint-renamed-before.txt
checkpoint-deleted.txt
```

Commit those two fixture files only inside the disposable clone:

```bash
git -C "$CHECKPOINT_FIXTURE/bauherrenmappe" add -- \
  checkpoint-renamed-before.txt checkpoint-deleted.txt
git -C "$CHECKPOINT_FIXTURE/bauherrenmappe" \
  -c user.name="Checkpoint Test" \
  -c user.email="checkpoint@example.invalid" \
  commit -m "test: add disposable checkpoint fixtures"
```

Then:

1. use `git mv` to rename `checkpoint-renamed-before.txt` to
   `checkpoint-renamed-after.txt`;
2. use `git rm` to delete `checkpoint-deleted.txt`;
3. use `apply_patch` to add `checkpoint-untracked.txt`;
4. use `apply_patch` to append one disposable sentence to the clone's
   `README.md`, leaving it unstaged.

Then run:

```bash
git -C "$CHECKPOINT_FIXTURE/bauherrenmappe" status --porcelain=v1 -z --untracked-files=all
```

Expected: NUL-delimited output contains staged, unstaged, renamed, deleted, and
untracked paths. Do not modify the original Bauherrenmappe checkout.

- [ ] **Step 7: Exercise first and repeated checkpoint storage**

Following the implemented `checkpoint` skill exactly:

1. derive `checkpoint-real-project` with
   `git symbolic-ref --quiet --short HEAD`;
2. collect the current NUL-delimited Git status;
3. create a safe semantic summary of the disposable changes;
4. run the Symfony Context Engine against the fixture root and a temporary DB:

```bash
python3 Symfony/memory-bank/scripts/context.py \
  --root "$CHECKPOINT_FIXTURE/bauherrenmappe" \
  --db "$CHECKPOINT_FIXTURE/context.db" \
  start \
  --task-id checkpoint-real-project \
  --goal "Checkpoint work on branch checkpoint-real-project" \
  --file checkpoint-untracked.txt \
  --json
python3 Symfony/memory-bank/scripts/context.py \
  --root "$CHECKPOINT_FIXTURE/bauherrenmappe" \
  --db "$CHECKPOINT_FIXTURE/context.db" \
  update \
  --task-id checkpoint-real-project \
  --progress "Added and reorganized disposable checkpoint fixtures." \
  --file checkpoint-untracked.txt \
  --json
```

Make one additional safe change in the disposable clone, collect status again,
and run a second `update` with:

```text
Updated the disposable checkpoint fixtures and verification notes.
```

Include every currently changed normalized path as repeated `--file` arguments
in both real executions, not only the illustrative path shown above.

- [ ] **Step 8: Verify the Bauherrenmappe Working Memory record**

Run:

```bash
python3 Symfony/memory-bank/scripts/context.py \
  --root "$CHECKPOINT_FIXTURE/bauherrenmappe" \
  --db "$CHECKPOINT_FIXTURE/context.db" \
  get --task-id checkpoint-real-project --json
```

Expected JSON:

- `task_id` equals `checkpoint-real-project`;
- `goal` equals `Checkpoint work on branch checkpoint-real-project`;
- `progress` equals the second semantic summary;
- `files` contains the union of first and second checkpoint paths without
  duplicates;
- no `complete` call or episode exists.

Run:

```bash
python3 Symfony/memory-bank/scripts/context.py \
  --root "$CHECKPOINT_FIXTURE/bauherrenmappe" \
  --db "$CHECKPOINT_FIXTURE/context.db" \
  status --json
```

Expected: `working` is `1` and `episodes` is `0`.

- [ ] **Step 9: Remove temporary evidence and prove the real checkout is unchanged**

Remove only the exact `mktemp` directory after validating that
`$CHECKPOINT_FIXTURE` matches `/tmp/tmp.*`:

```bash
case "$CHECKPOINT_FIXTURE" in
  /tmp/tmp.*) rm -rf -- "$CHECKPOINT_FIXTURE" ;;
  *) echo "Refusing unsafe temporary-directory removal" >&2; exit 1 ;;
esac
```

Then compare the original repository state:

```bash
git -C /home/aliaksei/Desktop/bauherrenmappe status --porcelain=v1 -z > /tmp/bauherrenmappe-status-after
git -C /home/aliaksei/Desktop/bauherrenmappe rev-parse HEAD > /tmp/bauherrenmappe-head-after
git -C /home/aliaksei/Desktop/bauherrenmappe write-tree > /tmp/bauherrenmappe-index-after
cmp /tmp/bauherrenmappe-status-before /tmp/bauherrenmappe-status-after
cmp /tmp/bauherrenmappe-head-before /tmp/bauherrenmappe-head-after
cmp /tmp/bauherrenmappe-index-before /tmp/bauherrenmappe-index-after
```

If the permanent database checksum was recorded, run:

```bash
sha256sum /home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db > /tmp/bauherrenmappe-context-after
cmp /tmp/bauherrenmappe-context-before /tmp/bauherrenmappe-context-after
```

Expected: every comparison exits `0`.

If the permanent database did not exist before the test, verify it still does
not exist:

```bash
test ! -e /home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db
```

Expected: the command exits `0`.

- [ ] **Step 10: Run final repository checks**

Run:

```bash
git diff --check
git status --short --branch
git log --oneline -5
```

Expected:

- `git diff --check` prints nothing;
- only the known unrelated `.langgraph_api/` and
  `ai_infrastructure_langgraph.egg-info/` remain untracked;
- the checkpoint implementation consists of the two scoped commits from Tasks
  1 and 2 after the already-committed spec and plan.
