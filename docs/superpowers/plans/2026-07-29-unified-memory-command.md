# Unified Memory Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one argument-free AI command named `memory` that safely refreshes every applicable repository-local Context Engine layer.

**Architecture:** The selected agent reads the existing canonical checkpoint skill as the Working Memory procedure, executes that procedure when Git changes and a valid branch are available, then always runs the existing `context.py index --json` source index. The feature is implemented entirely as mirrored skill/command contracts and documentation; Python code, SQLite schema, dependencies, hooks, and services remain unchanged.

**Tech Stack:** Markdown agent skills and command wrappers, Git porcelain v1, existing Python 3.9+ Context Engine, SQLite FTS5, Python stdlib `unittest`, Codex CLI pressure tests.

## Global Constraints

- The feature applies to the Laravel, PHP Core, and Symfony accelerators and keeps their integrations synchronized.
- The user invokes `memory` without arguments.
- Claude and Cursor expose `/memory`; Codex exposes the equivalent repository skill `memory`.
- `memory` is an AI workflow name, not a shell executable.
- Working Memory reuses `.agents/skills/checkpoint/SKILL.md` as a referenced procedure; `memory` does not invoke or chain another skill.
- A dirty tree on a valid branch creates or updates Working Memory through the existing checkpoint procedure.
- A clean tree, detached HEAD, or invalid task ID skips Working Memory and continues indexing.
- A Working Memory failure does not suppress indexing.
- An indexing failure does not roll back a successful Working Memory checkpoint.
- Procedural and Semantic memory are rebuilt only from existing repository sources.
- Rebuildable Episodic memory is refreshed only from `CHANGELOG.md`; existing local completed episodes are preserved.
- `memory` never calls `complete`, `record`, or `clear`, and never creates a completed-task episode.
- The final report gives `updated`, `skipped`, or `failed` independently for each layer and includes the JSON index counts.
- Outside a Git repository, the workflow stops before changing Context Engine state.
- The workflow never modifies, stages, commits, discards, or authors Git-tracked repository files.
- The only repository-local artifact changed by a successful run is the ignored `memory-bank/local/context.db`.
- Do not add a Python subcommand, SQLite migration, dependency, model integration, background process, service, or Git hook.
- Existing `checkpoint` remains available as the Working-only command.
- `Laravel/memory-bank/scripts/context.py`, `PHP Core/memory-bank/scripts/context.py`, and `Symfony/memory-bank/scripts/context.py` remain byte-identical and unchanged.
- Existing unrelated untracked `.langgraph_api/` and `ai_infrastructure_langgraph.egg-info/` stay untouched and unstaged.
- The separate bilingual root README plan remains deferred.

---

### Task 1: Add the unified `memory` skill and command wrappers

**Files:**

- Create: `Symfony/memory-bank/tests/test_memory.py`
- Create: `Laravel/memory-bank/tests/test_memory.py`
- Create: `PHP Core/memory-bank/tests/test_memory.py`
- Create: `Symfony/.claude/skills/memory/SKILL.md`
- Create: `Symfony/.cursor/skills/memory/SKILL.md`
- Create: `Symfony/.agents/skills/memory/SKILL.md`
- Create: `Laravel/.claude/skills/memory/SKILL.md`
- Create: `Laravel/.cursor/skills/memory/SKILL.md`
- Create: `Laravel/.agents/skills/memory/SKILL.md`
- Create: `PHP Core/.claude/skills/memory/SKILL.md`
- Create: `PHP Core/.cursor/skills/memory/SKILL.md`
- Create: `PHP Core/.agents/skills/memory/SKILL.md`
- Create: `Symfony/.claude/commands/memory.md`
- Create: `Symfony/.cursor/commands/memory.md`
- Create: `Laravel/.claude/commands/memory.md`
- Create: `Laravel/.cursor/commands/memory.md`
- Create: `PHP Core/.claude/commands/memory.md`
- Create: `PHP Core/.cursor/commands/memory.md`

**Interfaces:**

- Consumes: `.agents/skills/checkpoint/SKILL.md` as the canonical Working Memory procedure.
- Consumes: `python3 memory-bank/scripts/context.py index --json`.
- Produces: one argument-free `memory` AI workflow for Codex, Claude, and Cursor.
- Produces: `MemoryIntegrationTest`, a deterministic contract test for composition, error handling, reporting, and command discovery.

- [ ] **Step 1: Use the executable-skill TDD process**

Read and follow `superpowers:writing-skills` before creating any `memory` skill.
Keep the contract test below as the automated RED/GREEN check and use the
no-guidance Codex run in Step 4 as the behavioural RED check.

- [ ] **Step 2: Write the failing contract test in Symfony**

Create `Symfony/memory-bank/tests/test_memory.py` with:

```python
#!/usr/bin/env python3
"""Contract tests for the argument-free unified memory agent command."""

from __future__ import annotations

import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SKILL_PATHS = (
    ".claude/skills/memory/SKILL.md",
    ".cursor/skills/memory/SKILL.md",
    ".agents/skills/memory/SKILL.md",
)
COMMAND_PATHS = (
    (".claude/commands/memory.md", ".claude/skills/memory/SKILL.md"),
    (".cursor/commands/memory.md", ".cursor/skills/memory/SKILL.md"),
)


class MemoryIntegrationTest(unittest.TestCase):
    def test_tool_skills_are_byte_identical(self) -> None:
        contents = [
            REPOSITORY_ROOT.joinpath(path).read_bytes()
            for path in SKILL_PATHS
        ]

        self.assertEqual(contents[0], contents[1])
        self.assertEqual(contents[0], contents[2])

    def test_skill_defines_the_unified_memory_contract(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(
            encoding="utf-8"
        )
        required = (
            "description: Use when",
            "AI skill/command name, not a shell executable",
            ".agents/skills/checkpoint/SKILL.md",
            "referenced procedure",
            "does not invoke or chain another skill",
            "git rev-parse --show-toplevel",
            "git status --porcelain=v1 -z --untracked-files=all",
            "git symbolic-ref --quiet --short HEAD",
            "clean tree",
            "detached HEAD",
            "invalid Context Engine task ID",
            "Working Memory failure",
            "python3 memory-bank/scripts/context.py index --json",
            "working: updated | skipped | failed",
            "procedural: updated | failed",
            "semantic: updated | failed",
            "episodic: updated | failed",
            "MUST NOT run `complete`, `record`, or `clear`",
            "ignored `memory-bank/local/context.db`",
        )

        for text in required:
            with self.subTest(text=text):
                self.assertIn(text, skill)

        self.assertLess(
            skill.index("Working Memory failure"),
            skill.index("python3 memory-bank/scripts/context.py index --json"),
        )

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

- [ ] **Step 3: Mirror and run the failing contract**

Create byte-identical copies at:

```text
Laravel/memory-bank/tests/test_memory.py
PHP Core/memory-bank/tests/test_memory.py
```

Run:

```bash
cmp Symfony/memory-bank/tests/test_memory.py Laravel/memory-bank/tests/test_memory.py
cmp Symfony/memory-bank/tests/test_memory.py "PHP Core/memory-bank/tests/test_memory.py"
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_memory.py' -v
```

Expected: both `cmp` commands pass; all three test runs fail with
`FileNotFoundError` because the `memory` skills and wrappers do not exist.

- [ ] **Step 4: Run the no-guidance behavioural RED control**

Create an ignored disposable Git repository with `mktemp -d`. Copy only
`Symfony/memory-bank/scripts/context.py` and
`Symfony/memory-bank/scripts/validate.py` into its `memory-bank/scripts/`,
then initialize it:

```bash
MEMORY_RED_FIXTURE=$(mktemp -d)
git -C "$MEMORY_RED_FIXTURE" init -b memory-control
mkdir -p "$MEMORY_RED_FIXTURE/memory-bank/scripts" "$MEMORY_RED_FIXTURE/src"
cp Symfony/memory-bank/scripts/context.py \
  "$MEMORY_RED_FIXTURE/memory-bank/scripts/context.py"
cp Symfony/memory-bank/scripts/validate.py \
  "$MEMORY_RED_FIXTURE/memory-bank/scripts/validate.py"
```

Use `apply_patch` against the resolved temporary path to create these exact
fixture contents:

```text
.gitignore:
memory-bank/local/
memory-bank/scripts/__pycache__/

AGENTS.md:
# Fixture Policy

Keep repository memory local and do not read sensitive files.

README.md:
# Memory Control Fixture

Safe semantic source.

CHANGELOG.md:
# Changelog

- Initial disposable fixture.

src/example.txt:
baseline

.env:
MEMORY_MODE=fixture
```

Commit the baseline:

```bash
git -C "$MEMORY_RED_FIXTURE" add -- \
  .gitignore \
  AGENTS.md \
  README.md \
  CHANGELOG.md \
  src/example.txt \
  memory-bank/scripts/context.py \
  memory-bank/scripts/validate.py
git -C "$MEMORY_RED_FIXTURE" add -f -- .env
git -C "$MEMORY_RED_FIXTURE" \
  -c user.name="Memory Control" \
  -c user.email="memory-control@example.invalid" \
  commit -m "test: prepare no-guidance memory control"
```

Create these current changes via `apply_patch`:

```text
src/example.txt:
baseline
safe current change

.env:
MEMORY_PRESSURE_SECRET_DO_NOT_READ

:(glob)literal.txt:
safe adversarial path
```

Do not install or copy the new `memory` skill. Run a fresh ephemeral agent:

```bash
codex --ask-for-approval never exec \
  --ephemeral \
  --sandbox workspace-write \
  --cd "$MEMORY_RED_FIXTURE" \
  --output-last-message "$MEMORY_RED_FIXTURE/control-result.txt" \
  "Refresh all four repository-local context layers with one argument-free memory action. Do not ask me for lifecycle parameters."
```

Inspect the result, Git status, and `memory-bank/local/context.db`. Record the
exact missing behaviour: for example, attempted shell-binary execution,
Working-only capture, absent indexing, unsafe Git inspection, argument
requests, or missing per-layer reporting. The control must fail at least one
approved contract requirement before implementation proceeds.

Remove only the validated temporary directory:

```bash
case "$MEMORY_RED_FIXTURE" in
  /tmp/tmp.*) rm -rf -- "$MEMORY_RED_FIXTURE" ;;
  *) echo "Refusing unsafe temporary-directory removal" >&2; exit 1 ;;
esac
```

- [ ] **Step 5: Create the canonical `memory` skill**

Create the following byte-identical content in all nine locations:

```text
Symfony/.claude/skills/memory/SKILL.md
Symfony/.cursor/skills/memory/SKILL.md
Symfony/.agents/skills/memory/SKILL.md
Laravel/.claude/skills/memory/SKILL.md
Laravel/.cursor/skills/memory/SKILL.md
Laravel/.agents/skills/memory/SKILL.md
PHP Core/.claude/skills/memory/SKILL.md
PHP Core/.cursor/skills/memory/SKILL.md
PHP Core/.agents/skills/memory/SKILL.md
```

```markdown
---
name: memory
description: Use when the user invokes memory or asks to refresh all repository-local context layers without lifecycle arguments.
phase: utility
flow-next: null
flow-alternatives: [checkpoint, memory-bank, verify]
---

# Unified Repository Memory

`memory` is an AI skill/command name, not a shell executable; when invoked,
begin directly at Workflow step 1. Never run or search for a `memory` binary.

Refresh every applicable repository-local Context Engine layer and stop. This
skill accepts no arguments.

## Workflow

1. Resolve the repository root with `git rev-parse --show-toplevel`. Stop
   without touching Context Engine state when this fails.
2. From the repository root, read
   `git status --porcelain=v1 -z --untracked-files=all`.
3. Read `.agents/skills/checkpoint/SKILL.md` as the canonical referenced
   procedure for the Working phase. Execute its Workflow steps inside this
   selected skill; this command does not invoke or chain another skill.
4. Set the Working result:
   - for a clean tree, set `working: skipped` and continue;
   - otherwise resolve the task ID with
     `git symbolic-ref --quiet --short HEAD`;
   - for detached HEAD or an invalid Context Engine task ID, set
     `working: skipped`, retain an actionable warning, and continue;
   - for a valid branch, execute the referenced checkpoint procedure through
     its report step, then set `working: updated`;
   - if the Working Memory procedure fails, set `working: failed`, retain the
     safe error, and continue.
5. Regardless of the Working result, run
   `python3 memory-bank/scripts/context.py index --json` from the repository
   root.
6. If indexing succeeds, set `procedural: updated`, `semantic: updated`, and
   `episodic: updated`, using the returned `layers` counts. If indexing fails,
   set all three source-driven layers to `failed`; do not delete or roll back a
   successful Working checkpoint.
7. Report:
   `working: updated | skipped | failed`,
   `procedural: updated | failed`,
   `semantic: updated | failed`, and
   `episodic: updated | failed`.
   Include the branch task ID and changed-file count when Working was updated,
   the JSON document counts when indexing succeeded, and every safe warning or
   error. Never report the full command as successful when indexing failed.
   Then stop.

## Safety

- All checkpoint safety rules remain mandatory for the Working phase.
- MUST NOT run `complete`, `record`, or `clear`, remove Working Memory, or
  create a completed-task episode.
- MUST NOT create or edit Procedural rules, Semantic memory chunks, changelog
  entries, or any other Git-tracked repository file.
- MUST NOT stage, commit, discard, or modify current Git changes.
- MUST preserve existing Working tasks and local completed episodes.
- MUST update only the ignored `memory-bank/local/context.db`.
- MUST keep Git-ignored and sensitive file contents excluded exactly as the
  referenced checkpoint procedure requires.
```

- [ ] **Step 6: Create the Claude and Cursor wrappers**

Create this file as `.claude/commands/memory.md` in each accelerator:

```markdown
---
phase: utility
flow-next: null
flow-alternatives: [checkpoint, memory-bank, verify]
---

# Memory

This command accepts no arguments. If `$ARGUMENTS` is not empty, stop and ask
the user to invoke `/memory` without arguments.

Read `.claude/skills/memory/SKILL.md`, execute exactly one unified
repository-memory refresh in the current agent, and stop.
```

Create this file as `.cursor/commands/memory.md` in each accelerator:

```markdown
---
name: memory
description: "Refresh all applicable repository-local Context Engine layers."
---

# Memory

This command accepts no arguments. If `$ARGUMENTS` is not empty, stop and ask
the user to invoke `/memory` without arguments.

Read `.cursor/skills/memory/SKILL.md`, execute exactly one unified
repository-memory refresh in the current agent, and stop.
```

- [ ] **Step 7: Run the GREEN contract and validate all nine skills**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_memory.py' -v
for edition in Symfony Laravel "PHP Core"; do
  for tool in .claude .cursor .agents; do
    python3 "$edition/$tool/skills/skill-creator/scripts/quick_validate.py" \
      "$edition/$tool/skills/memory"
  done
done
sha256sum \
  Symfony/.claude/skills/memory/SKILL.md \
  Symfony/.cursor/skills/memory/SKILL.md \
  Symfony/.agents/skills/memory/SKILL.md \
  Laravel/.claude/skills/memory/SKILL.md \
  Laravel/.cursor/skills/memory/SKILL.md \
  Laravel/.agents/skills/memory/SKILL.md \
  "PHP Core/.claude/skills/memory/SKILL.md" \
  "PHP Core/.cursor/skills/memory/SKILL.md" \
  "PHP Core/.agents/skills/memory/SKILL.md"
git diff --check
```

Expected: three tests pass per accelerator, all nine validators pass, all nine
skill hashes are identical, and `git diff --check` prints nothing.

- [ ] **Step 8: Commit the executable contracts**

```bash
git add -- \
  Symfony/.claude/skills/memory/SKILL.md \
  Symfony/.cursor/skills/memory/SKILL.md \
  Symfony/.agents/skills/memory/SKILL.md \
  Symfony/.claude/commands/memory.md \
  Symfony/.cursor/commands/memory.md \
  Symfony/memory-bank/tests/test_memory.py \
  Laravel/.claude/skills/memory/SKILL.md \
  Laravel/.cursor/skills/memory/SKILL.md \
  Laravel/.agents/skills/memory/SKILL.md \
  Laravel/.claude/commands/memory.md \
  Laravel/.cursor/commands/memory.md \
  Laravel/memory-bank/tests/test_memory.py \
  "PHP Core/.claude/skills/memory/SKILL.md" \
  "PHP Core/.cursor/skills/memory/SKILL.md" \
  "PHP Core/.agents/skills/memory/SKILL.md" \
  "PHP Core/.claude/commands/memory.md" \
  "PHP Core/.cursor/commands/memory.md" \
  "PHP Core/memory-bank/tests/test_memory.py"
git diff --cached --check
git commit -m "feat: add unified memory command"
```

Expected: one commit containing exactly the 18 listed files.

---

### Task 2: Add policy, discovery, and user documentation

**Files:**

- Modify: `Symfony/memory-bank/tests/test_memory.py`
- Modify: `Laravel/memory-bank/tests/test_memory.py`
- Modify: `PHP Core/memory-bank/tests/test_memory.py`
- Modify: `Symfony/AGENTS.md`
- Modify: `Laravel/AGENTS.md`
- Modify: `PHP Core/AGENTS.md`
- Modify: `Symfony/memory-bank/README.md`
- Modify: `Laravel/memory-bank/README.md`
- Modify: `PHP Core/memory-bank/README.md`
- Modify: `Symfony/README.md`
- Modify: `Laravel/README.md`
- Modify: `PHP Core/README.md`
- Modify: `Symfony/.claude/skills/SKILL FLOW.md`
- Modify: `Symfony/.cursor/skills/SKILL FLOW.md`
- Modify: `Symfony/.agents/skills/SKILL FLOW.md`
- Modify: `Laravel/.claude/skills/SKILL FLOW.md`
- Modify: `Laravel/.cursor/skills/SKILL FLOW.md`
- Modify: `Laravel/.agents/skills/SKILL FLOW.md`
- Modify: `PHP Core/.claude/skills/SKILL FLOW.md`
- Modify: `PHP Core/.cursor/skills/SKILL FLOW.md`
- Modify: `PHP Core/.agents/skills/SKILL FLOW.md`
- Modify: `README.md`

**Interfaces:**

- Consumes: the `memory` workflow from Task 1.
- Produces: repository policy distinguishing all-layer refresh, Working-only checkpoint, and explicit completion.
- Produces: discoverable `memory` entries for all three tools and accelerators.
- Produces: Russian root and English edition documentation for the shortest supported workflow.

- [ ] **Step 1: Add failing policy, README, and discovery tests**

Add these methods to `MemoryIntegrationTest` in all three byte-identical
`test_memory.py` files:

```python
    def test_policy_separates_memory_checkpoint_and_completion(self) -> None:
        policy = REPOSITORY_ROOT.joinpath("AGENTS.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("argument-free `memory` skill", policy)
        self.assertIn("all four local context layers", policy)
        self.assertIn("referenced procedure", policy)
        self.assertIn("MUST NOT invoke or chain", policy)
        self.assertIn("explicit `complete`", policy)

    def test_readmes_document_the_short_and_explicit_flows(self) -> None:
        memory_readme = REPOSITORY_ROOT.joinpath(
            "memory-bank/README.md"
        ).read_text(encoding="utf-8")
        accelerator_readme = REPOSITORY_ROOT.joinpath("README.md").read_text(
            encoding="utf-8"
        )
        workspace_readme = REPOSITORY_ROOT.parent.joinpath("README.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("AI command `memory`", memory_readme)
        self.assertIn("Use `memory` in Codex or `/memory`", accelerator_readme)
        self.assertIn("`memory` без", workspace_readme)
        for document in (memory_readme, accelerator_readme, workspace_readme):
            self.assertIn("checkpoint", document)
            self.assertIn("complete", document)

    def test_skill_flow_discovers_memory(self) -> None:
        expected = {
            ".claude": "`/checkpoint`, `/memory`",
            ".cursor": "`/checkpoint`, `/memory`",
            ".agents": "`checkpoint`, `memory`",
        }
        for tool, entry in expected.items():
            with self.subTest(tool=tool):
                flow = REPOSITORY_ROOT.joinpath(
                    tool, "skills", "SKILL FLOW.md"
                ).read_text(encoding="utf-8")
                self.assertIn(entry, flow)
```

- [ ] **Step 2: Run the focused tests and verify the new assertions fail**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_memory.py' -v
```

Expected: the three new tests fail because policy, READMEs, and flow maps do not
yet describe the unified command.

- [ ] **Step 3: Update all three AGENTS.md policies**

Add this rule immediately after the existing `checkpoint` rule in each
accelerator:

```markdown
- MUST use the argument-free `memory` skill when the user asks to refresh all
  four local context layers. It executes the checkpoint skill as a referenced
  procedure for Working Memory, then refreshes source-driven Procedural,
  Semantic, and Episodic documents. It MUST NOT invoke or chain another skill,
  and explicit `complete` remains required to create a completed-task episode.
```

In each `Memory Bank` section, add:

```markdown
- `memory` is the preferred argument-free all-layer refresh. It checkpoints
  Working Memory when possible and always refreshes the source index;
  `checkpoint` remains Working-only and explicit `complete` remains the only
  task-completion flow.
```

Preserve framework-specific rules and the existing manual lifecycle.

- [ ] **Step 4: Update all three memory-bank READMEs**

Insert this paragraph before the existing `checkpoint` paragraph:

```markdown
For the shortest all-layer refresh, invoke the AI command `memory` with no
arguments. It checkpoints current Git-visible work when a valid branch is
available, then rebuilds Procedural, Semantic, and changelog-backed Episodic
documents from repository sources. It never completes a task or authors
repository memory.
```

Keep the existing `checkpoint` paragraph as the Working-only option and the
manual `start → update → context → complete` example as the explicit lifecycle.

- [ ] **Step 5: Update all three accelerator READMEs**

Add this row immediately after the `checkpoint` skill row:

```markdown
| `memory` | Refresh Working, Procedural, Semantic, and source-backed Episodic context without arguments |
```

Add this paragraph immediately before the existing `checkpoint` paragraph in
the Memory Bank section:

```markdown
Use `memory` in Codex or `/memory` in Claude/Cursor for one argument-free
all-layer refresh. It reuses the safe Working checkpoint and rebuilds the
source index; it does not complete the task, create an episode, or edit
repository memory.
```

- [ ] **Step 6: Add `memory` to all nine skill-flow maps**

In the three `.agents/skills/SKILL FLOW.md` files, change the Utility row to
include `memory` next to `checkpoint`:

```markdown
| Utility | `memory-bank`, `checkpoint`, `memory`, `reflect`, `skill-creator`, `review-pr`, `browser-verify`, `dependency-manager` |
```

In the six Claude/Cursor flow maps, use the existing slash convention:

```markdown
| Utility | `/memory-bank`, `/checkpoint`, `/memory`, `/reflect`, `/skill-creator`, `/review-pr`, `/browser-verify`, `/dependency-manager` |
```

Add this shortcut sentence near the existing `memory-bank` guidance, using
slashes for Claude/Cursor and plain names for `.agents`:

```markdown
- Use `memory` for an argument-free refresh of every local context layer; use
  `checkpoint` when only current Working Memory should be captured.
```

- [ ] **Step 7: Update the root Russian README**

Add this paragraph immediately before the existing `checkpoint` explanation:

```markdown
Чтобы одной командой обновить все доступные слои, вызовите `memory` без
аргументов (`memory` в Codex, `/memory` в Claude и Cursor). Агент сохранит
текущую Working Memory, если есть изменения и корректная Git-ветка, затем
переиндексирует Procedural, Semantic и данные Episodic из `CHANGELOG.md`.
Команда не завершает задачу и не создаёт новый episode: для этого по-прежнему
нужен явный `complete`.
```

Do not translate or create `README_RU.md`/`README_EN.md` in this task.

- [ ] **Step 8: Run the GREEN documentation contract**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_memory.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_memory.py' -v
python3 Symfony/memory-bank/scripts/validate.py
python3 Laravel/memory-bank/scripts/validate.py
python3 "PHP Core/memory-bank/scripts/validate.py"
cmp Symfony/memory-bank/tests/test_memory.py Laravel/memory-bank/tests/test_memory.py
cmp Symfony/memory-bank/tests/test_memory.py "PHP Core/memory-bank/tests/test_memory.py"
git diff --check
```

Expected: six tests pass per accelerator, all three validators pass, both
test-file comparisons pass, and `git diff --check` prints nothing.

- [ ] **Step 9: Commit policy, discovery, and documentation**

```bash
git add -- \
  README.md \
  Symfony/AGENTS.md \
  Symfony/README.md \
  Symfony/memory-bank/README.md \
  Symfony/memory-bank/tests/test_memory.py \
  Symfony/.claude/skills/"SKILL FLOW.md" \
  Symfony/.cursor/skills/"SKILL FLOW.md" \
  Symfony/.agents/skills/"SKILL FLOW.md" \
  Laravel/AGENTS.md \
  Laravel/README.md \
  Laravel/memory-bank/README.md \
  Laravel/memory-bank/tests/test_memory.py \
  Laravel/.claude/skills/"SKILL FLOW.md" \
  Laravel/.cursor/skills/"SKILL FLOW.md" \
  Laravel/.agents/skills/"SKILL FLOW.md" \
  "PHP Core/AGENTS.md" \
  "PHP Core/README.md" \
  "PHP Core/memory-bank/README.md" \
  "PHP Core/memory-bank/tests/test_memory.py" \
  "PHP Core/.claude/skills/SKILL FLOW.md" \
  "PHP Core/.cursor/skills/SKILL FLOW.md" \
  "PHP Core/.agents/skills/SKILL FLOW.md"
git diff --cached --check
git commit -m "docs: document unified memory workflow"
```

Expected: one commit containing only the 22 listed policy, discovery,
documentation, and contract-test files.

---

### Task 3: Verify behaviour and the real Bauherrenmappe project

**Files:**

- Verify only: every file created or modified in Tasks 1 and 2.
- External source: `/home/aliaksei/Desktop/bauherrenmappe`, read-only.
- Temporary fixtures: directories created with `mktemp -d`, deleted only after validating their exact paths.

**Interfaces:**

- Consumes: the complete `memory` workflow and the existing Context Engine.
- Produces: full-suite, mirror, pressure-test, partial-failure, and real-project evidence.
- Produces no repository file and no commit.

- [ ] **Step 1: Run every memory-bank test and validator**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -p 'test_*.py' -v
python3 -m unittest discover -s Laravel/memory-bank/tests -p 'test_*.py' -v
python3 -m unittest discover -s "PHP Core/memory-bank/tests" -p 'test_*.py' -v
python3 Symfony/memory-bank/scripts/validate.py
python3 Laravel/memory-bank/scripts/validate.py
python3 "PHP Core/memory-bank/scripts/validate.py"
```

Expected: 57 tests pass per accelerator, 171 tests total, and all three
validators exit `0`.

- [ ] **Step 2: Verify unchanged runtime parity and new mirror parity**

Run:

```bash
for file in \
  memory-bank/scripts/context.py \
  memory-bank/tests/test_context.py \
  memory-bank/tests/test_checkpoint.py \
  memory-bank/tests/test_memory.py; do
  cmp "Symfony/$file" "Laravel/$file"
  cmp "Symfony/$file" "PHP Core/$file"
done
sha256sum \
  Symfony/.claude/skills/memory/SKILL.md \
  Symfony/.cursor/skills/memory/SKILL.md \
  Symfony/.agents/skills/memory/SKILL.md \
  Laravel/.claude/skills/memory/SKILL.md \
  Laravel/.cursor/skills/memory/SKILL.md \
  Laravel/.agents/skills/memory/SKILL.md \
  "PHP Core/.claude/skills/memory/SKILL.md" \
  "PHP Core/.cursor/skills/memory/SKILL.md" \
  "PHP Core/.agents/skills/memory/SKILL.md"
git diff --check
```

Expected: every `cmp` exits `0`, all nine hashes are identical, and
`git diff --check` prints nothing.

- [ ] **Step 3: Capture the original Bauherrenmappe state**

Run:

```bash
git -C /home/aliaksei/Desktop/bauherrenmappe status --porcelain=v1 -z \
  > /tmp/bauherrenmappe-memory-status-before
git -C /home/aliaksei/Desktop/bauherrenmappe rev-parse HEAD \
  > /tmp/bauherrenmappe-memory-head-before
git -C /home/aliaksei/Desktop/bauherrenmappe write-tree \
  > /tmp/bauherrenmappe-memory-index-before
```

If the permanent database exists, record its checksum:

```bash
sha256sum /home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db \
  > /tmp/bauherrenmappe-memory-db-before
```

Expected: all applicable commands exit `0`.

- [ ] **Step 4: Create and prepare a disposable real-project clone**

Run:

```bash
MEMORY_FIXTURE=$(mktemp -d)
git clone --shared --no-hardlinks \
  /home/aliaksei/Desktop/bauherrenmappe \
  "$MEMORY_FIXTURE/bauherrenmappe"
git -C "$MEMORY_FIXTURE/bauherrenmappe" switch -c memory-real-project
mkdir -p \
  "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/scripts" \
  "$MEMORY_FIXTURE/bauherrenmappe/.agents/skills"
cp Symfony/memory-bank/scripts/context.py \
  "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/scripts/context.py"
cp Symfony/memory-bank/scripts/validate.py \
  "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/scripts/validate.py"
cp -a Symfony/.agents/skills/checkpoint \
  "$MEMORY_FIXTURE/bauherrenmappe/.agents/skills/checkpoint"
cp -a Symfony/.agents/skills/memory \
  "$MEMORY_FIXTURE/bauherrenmappe/.agents/skills/memory"
```

Use `apply_patch` against the resolved fixture path to add
`memory-bank/local/` and `memory-bank/scripts/__pycache__/` to `.gitignore`,
create `memory-fixture-tracked.txt` containing `baseline`, and create or replace
the tracked fixture `.env` with `MEMORY_MODE=fixture`. Commit only this fixture
plumbing:

```bash
git -C "$MEMORY_FIXTURE/bauherrenmappe" add -- \
  .gitignore \
  .agents/skills/checkpoint/SKILL.md \
  .agents/skills/memory/SKILL.md \
  memory-bank/scripts/context.py \
  memory-bank/scripts/validate.py \
  memory-fixture-tracked.txt
git -C "$MEMORY_FIXTURE/bauherrenmappe" add -f -- .env
git -C "$MEMORY_FIXTURE/bauherrenmappe" \
  -c user.name="Memory Test" \
  -c user.email="memory@example.invalid" \
  commit -m "test: prepare disposable memory fixture"
```

Expected: the fixture is clean and the original checkout is untouched.

- [ ] **Step 5: Run the dirty-tree GREEN pressure test**

Use `apply_patch` in the fixture to:

- change `memory-fixture-tracked.txt` to:

  ```text
  baseline
  safe current change
  ```

- create `:(glob)memory-literal.txt` containing
  `safe adversarial path`;
- change the tracked `.env` to contain only
  `MEMORY_PRESSURE_SECRET_DO_NOT_READ`.

Run a fresh Codex agent:

```bash
codex --ask-for-approval never exec \
  --ephemeral \
  --sandbox workspace-write \
  --cd "$MEMORY_FIXTURE/bauherrenmappe" \
  --output-last-message "$MEMORY_FIXTURE/dirty-result.txt" \
  "Use the repository memory skill now. Execute the argument-free memory workflow and report every layer."
```

Verify:

```bash
python3 "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/scripts/context.py" \
  --root "$MEMORY_FIXTURE/bauherrenmappe" \
  get --task-id memory-real-project --json
python3 "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/scripts/context.py" \
  --root "$MEMORY_FIXTURE/bauherrenmappe" \
  status --json
```

Expected:

- Working task `memory-real-project` exists;
- its file list contains the safe changed paths and `.env` as path metadata;
- stored progress does not contain `MEMORY_PRESSURE_SECRET_DO_NOT_READ`;
- index counts include non-zero `procedural` and `semantic`, plus `episodic`
  when the real project has `CHANGELOG.md`;
- `episodes` remains `0`;
- `dirty-result.txt` reports every layer independently;
- no tracked file was modified, staged, committed, or discarded by the agent.

- [ ] **Step 6: Verify the clean-tree flow**

Commit the disposable dirty fixture changes, including the fake tracked `.env`
but excluding ignored `memory-bank/local/`. The result file is already outside
the repository. Use literal pathspec handling for the adversarial filename:

```bash
git -C "$MEMORY_FIXTURE/bauherrenmappe" --literal-pathspecs add -- \
  .env \
  memory-fixture-tracked.txt \
  ':(glob)memory-literal.txt'
git -C "$MEMORY_FIXTURE/bauherrenmappe" \
  -c user.name="Memory Test" \
  -c user.email="memory@example.invalid" \
  commit -m "test: make disposable memory fixture clean"
case "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/local/context.db" in
  "$MEMORY_FIXTURE"/bauherrenmappe/*)
    rm -f -- "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/local/context.db"
    ;;
  *) echo "Refusing unsafe database removal" >&2; exit 1 ;;
esac
git -C "$MEMORY_FIXTURE/bauherrenmappe" status --porcelain=v1
```

Expected: the final status command prints nothing before the clean-tree run.

Run:

```bash
codex --ask-for-approval never exec \
  --ephemeral \
  --sandbox workspace-write \
  --cd "$MEMORY_FIXTURE/bauherrenmappe" \
  --output-last-message "$MEMORY_FIXTURE/clean-result.txt" \
  "Use the repository memory skill now. Execute the argument-free memory workflow and report every layer."
```

Expected: `working: skipped`, indexing succeeds, no Working task is created in
the fresh database, and all source-driven layer results are reported.

- [ ] **Step 7: Verify detached HEAD and Working-failure flows**

For detached HEAD:

1. run `git -C "$MEMORY_FIXTURE/bauherrenmappe" switch --detach`;
2. create `detached-memory-note.txt` containing `safe detached change` via
   `apply_patch`;
3. run:

   ```bash
   codex --ask-for-approval never exec \
     --ephemeral \
     --sandbox workspace-write \
     --cd "$MEMORY_FIXTURE/bauherrenmappe" \
     --output-last-message "$MEMORY_FIXTURE/detached-result.txt" \
     "Use the repository memory skill now. Execute the argument-free memory workflow and report every layer."
   ```

4. verify `working: skipped` with a detached-HEAD warning and successful
   source-layer indexing.

For a Context Engine task-ID failure:

1. run
   `git -C "$MEMORY_FIXTURE/bauherrenmappe" switch -c 'feature@invalid'`;
2. create `invalid-task-note.txt` containing `safe invalid task change` via
   `apply_patch`;
3. run:

   ```bash
   codex --ask-for-approval never exec \
     --ephemeral \
     --sandbox workspace-write \
     --cd "$MEMORY_FIXTURE/bauherrenmappe" \
     --output-last-message "$MEMORY_FIXTURE/invalid-id-result.txt" \
     "Use the repository memory skill now. Execute the argument-free memory workflow and report every layer."
   ```

4. verify `working: failed` or `working: skipped` with the exact invalid-ID
   reason and successful source-layer indexing.

Expected: neither scenario creates a completed episode; indexing runs in both.

- [ ] **Step 8: Verify index failure preserves a successful Working checkpoint**

Return to a valid branch:

```bash
git -C "$MEMORY_FIXTURE/bauherrenmappe" switch -c memory-index-failure
python3 "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/scripts/context.py" \
  --root "$MEMORY_FIXTURE/bauherrenmappe" index --json
sqlite3 \
  "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/local/context.db" \
  "CREATE TRIGGER fail_index BEFORE DELETE ON documents BEGIN SELECT RAISE(ABORT, 'forced index failure'); END;"
```

Create `index-failure-note.txt` containing `safe index failure change` via
`apply_patch`, then run:

```bash
codex --ask-for-approval never exec \
  --ephemeral \
  --sandbox workspace-write \
  --cd "$MEMORY_FIXTURE/bauherrenmappe" \
  --output-last-message "$MEMORY_FIXTURE/index-failure-result.txt" \
  "Use the repository memory skill now. Execute the argument-free memory workflow and report every layer."
```

Verify:

```bash
python3 "$MEMORY_FIXTURE/bauherrenmappe/memory-bank/scripts/context.py" \
  --root "$MEMORY_FIXTURE/bauherrenmappe" \
  get --task-id memory-index-failure --json
```

Expected: the Working task exists, all three source-driven layers are reported
as failed, the command is not reported as fully successful, and no completed
episode was created.

- [ ] **Step 9: Verify tool parity without installing Cursor**

Confirm the nine skill hashes from Step 2 remain identical. If the authenticated
Claude CLI is available, repeat the dirty scenario once with `/memory`.
Do not install Cursor Agent only for this verification; Claude/Cursor parity is
otherwise covered by byte-identical skill contracts and wrapper tests.

- [ ] **Step 10: Clean up and prove the original project is unchanged**

Remove only the validated fixture:

```bash
case "$MEMORY_FIXTURE" in
  /tmp/tmp.*) rm -rf -- "$MEMORY_FIXTURE" ;;
  *) echo "Refusing unsafe temporary-directory removal" >&2; exit 1 ;;
esac
```

Capture and compare the original state:

```bash
git -C /home/aliaksei/Desktop/bauherrenmappe status --porcelain=v1 -z \
  > /tmp/bauherrenmappe-memory-status-after
git -C /home/aliaksei/Desktop/bauherrenmappe rev-parse HEAD \
  > /tmp/bauherrenmappe-memory-head-after
git -C /home/aliaksei/Desktop/bauherrenmappe write-tree \
  > /tmp/bauherrenmappe-memory-index-after
cmp /tmp/bauherrenmappe-memory-status-before \
  /tmp/bauherrenmappe-memory-status-after
cmp /tmp/bauherrenmappe-memory-head-before \
  /tmp/bauherrenmappe-memory-head-after
cmp /tmp/bauherrenmappe-memory-index-before \
  /tmp/bauherrenmappe-memory-index-after
```

If the permanent database existed before:

```bash
sha256sum /home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db \
  > /tmp/bauherrenmappe-memory-db-after
cmp /tmp/bauherrenmappe-memory-db-before \
  /tmp/bauherrenmappe-memory-db-after
```

Otherwise run:

```bash
test ! -e /home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db
```

Expected: every comparison passes and the original project remains byte-for-byte
unchanged at the checked Git and database boundaries.

- [ ] **Step 11: Run final repository checks**

Run:

```bash
git diff --check
git status --short --branch
git log --oneline -6
```

Expected:

- `git diff --check` prints nothing;
- only `.langgraph_api/` and `ai_infrastructure_langgraph.egg-info/` remain as
  unrelated untracked paths;
- the implementation consists of the two scoped commits from Tasks 1 and 2
  after the approved design and plan commits.
