# Four-Layer Context Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the repository-local Context Engine with explicit working, episodic, semantic, and procedural memory in one ignored SQLite database.

**Architecture:** Keep the existing dependency-free CLI and SQLite FTS5 store. Repository files are classified into procedural, semantic, or episodic documents during indexing; temporary working tasks use a regular SQLite table keyed by an explicit `task-id`; `complete` atomically converts one working task into an episode.

**Tech Stack:** Python 3.9+ standard library, SQLite FTS5, `argparse`, `unittest`, existing memory-bank validator and secret patterns

## Global Constraints

- Keep `Laravel/`, `PHP Core/`, and `Symfony/` self-contained.
- Add no package, network service, embedding model, MCP adapter, or LangGraph dependency.
- Keep the database at ignored `memory-bank/local/context.db` by default.
- Keep production scripts and tests byte-identical across all three accelerators.
- Preserve existing episodes during every schema migration.
- Never store raw prompts, responses, logs, secrets, credentials, or customer data.
- Require caller-supplied task IDs matching `[A-Za-z0-9][A-Za-z0-9._/-]{0,127}`.
- Keep existing `index`, `search`, `record`, and `status` commands compatible.
- Do not modify `Infrastructure-Creator`.

---

### Task 1: Classify Repository Sources Into Durable Memory Layers

**Files:**
- Modify: `Symfony/memory-bank/scripts/context.py`
- Test: `Symfony/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: existing `connect()`, `discover_documents()`, `index_repository()`, and `search_documents()`
- Produces: document tuples `(path, layer, kind, title, content)` and `search_documents(connection: sqlite3.Connection, query: str, limit: int, layer: Optional[str] = None) -> list[dict[str, object]]`

- [ ] **Step 1: Write the failing layer-classification and skill-deduplication test**

Add this integration test to `ContextEngineTest`:

```python
def test_index_classifies_layers_and_deduplicates_mirrored_skills(self) -> None:
    self.repository.joinpath("AGENTS.md").write_text(
        "# Policy\n\nUse the cobalt review procedure.\n",
        encoding="utf-8",
    )
    self.repository.joinpath("README.md").write_text(
        "# Domain\n\nInvoices follow the amber ownership rule.\n",
        encoding="utf-8",
    )
    self.repository.joinpath("CHANGELOG.md").write_text(
        "# Changes\n\nAdded the violet retry boundary.\n",
        encoding="utf-8",
    )
    for tool in (".agents", ".claude", ".cursor"):
        skill = self.repository / tool / "skills/review/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text(
            "# Review\n\nRun the indigo verification procedure.\n",
            encoding="utf-8",
        )

    indexed = self.run_context("index", "--json")
    self.assertEqual(0, indexed.returncode, indexed.stderr)
    self.assertEqual(
        {"procedural": 2, "semantic": 1, "episodic": 1},
        json.loads(indexed.stdout)["layers"],
    )

    searched = self.run_context("search", "indigo", "--json")
    documents = json.loads(searched.stdout)["documents"]
    self.assertEqual(1, len(documents))
    self.assertEqual("procedural", documents[0]["layer"])
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
python3 -m unittest \
  Symfony/memory-bank/tests/test_context.py \
  -k test_index_classifies_layers_and_deduplicates_mirrored_skills -v
```

Expected: failure because indexed documents do not have a `layer`, skill files
are not indexed, and the JSON result has no `layers` counts.

- [ ] **Step 3: Add the layer-aware disposable document schema**

Change the FTS5 schema to:

```sql
CREATE VIRTUAL TABLE documents USING fts5(
    path UNINDEXED,
    layer UNINDEXED,
    kind UNINDEXED,
    title,
    content,
    tokenize = 'unicode61'
)
```

Before using an existing `documents` table, inspect
`PRAGMA table_info(documents)`. If `layer` is absent, drop and recreate only the
disposable document table inside the existing explicit schema transaction.
Do not drop `episodes`.

- [ ] **Step 4: Classify bounded sources and deduplicate mirrored skills**

Represent source rules as `(layer, kind, pattern)` tuples:

```python
SOURCE_PATTERNS = (
    ("procedural", "policy", "AGENTS.md"),
    ("procedural", "policy", "CLAUDE.md"),
    ("procedural", "skill", ".agents/skills/**/*.md"),
    ("procedural", "skill", ".claude/skills/**/*.md"),
    ("procedural", "skill", ".cursor/skills/**/*.md"),
    ("procedural", "skill", ".codex/skills/**/*.md"),
    ("semantic", "overview", "README.md"),
    ("semantic", "spec", "specs/**/*.md"),
    ("semantic", "spec", "docs/**/*.md"),
    ("semantic", "memory", "memory-bank/chunks/*.md"),
    ("semantic", "task", "tasks/**/*.md"),
    ("semantic", "capability", "Task/Epics/**/*.md"),
    ("episodic", "changelog", "CHANGELOG.md"),
)
```

For skill files, derive the logical key from the path after `skills/`, such as
`review/SKILL.md`, and keep only the first match in source-pattern order.
Continue applying canonical metadata and secret validation to active memory
chunks.

Update inserts, searches, JSON results, and index counts to include `layer`.
Update `status` with document counts grouped by procedural, semantic, and
episodic layer.
Keep general search output keys `query`, `documents`, and `episodes`.

- [ ] **Step 5: Run the focused and complete Symfony suites**

Run:

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -v
```

Expected: all existing tests and the new classification test pass.

- [ ] **Step 6: Commit the classification**

```bash
git add -- \
  Symfony/memory-bank/scripts/context.py \
  Symfony/memory-bank/tests/test_context.py
git commit -m "feat(symfony): classify context memory layers"
```

### Task 2: Add Explicit Working Memory By Task ID

**Files:**
- Modify: `Symfony/memory-bank/scripts/context.py`
- Test: `Symfony/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: `SECRET_PATTERNS`, SQLite connection, JSON-list encoding
- Produces:
  - `validate_task_id(task_id: str) -> str`
  - `start_working_task(connection: sqlite3.Connection, task_id: str, goal: str, files: list[str], sources: list[str]) -> dict[str, object]`
  - `update_working_task(connection: sqlite3.Connection, task_id: str, progress: Optional[str], next_steps: list[str], files: list[str], sources: list[str]) -> dict[str, object]`
  - `get_working_task(connection: sqlite3.Connection, task_id: str) -> dict[str, object]`
  - `clear_working_task(connection: sqlite3.Connection, task_id: str) -> None`

- [ ] **Step 1: Write failing working-memory lifecycle tests**

Add tests that:

```python
first = self.run_context(
    "start",
    "--task-id", "BAUMAS-133",
    "--goal", "Invalidate other password sessions.",
    "--file", "src/GraphQL/Resolver/ChangePasswordResolver.php",
    "--json",
)
second = self.run_context(
    "start",
    "--task-id", "BAUMAS-134",
    "--goal", "Add an independent audit check.",
    "--json",
)
self.assertEqual(0, first.returncode, first.stderr)
self.assertEqual(0, second.returncode, second.stderr)

updated = self.run_context(
    "update",
    "--task-id", "BAUMAS-133",
    "--progress", "Two-session regression passes.",
    "--next-step", "Verify remember-me invalidation.",
    "--file", "tests/Integration/GraphQL/ChangePasswordTest.php",
    "--json",
)
self.assertEqual(0, updated.returncode, updated.stderr)

status = json.loads(self.run_context("status", "--json").stdout)
self.assertEqual(2, status["working"])

cleared = self.run_context(
    "clear", "--task-id", "BAUMAS-133", "--json"
)
self.assertEqual(0, cleared.returncode, cleared.stderr)
status = json.loads(self.run_context("status", "--json").stdout)
self.assertEqual(1, status["working"])
```

Add explicit boundary tests:

```python
def test_working_rejects_duplicate_unknown_and_invalid_tasks(self) -> None:
    self.assertEqual(
        0,
        self.run_context(
            "start", "--task-id", "TASK-1", "--goal", "First task"
        ).returncode,
    )
    duplicate = self.run_context(
        "start", "--task-id", "TASK-1", "--goal", "Duplicate task"
    )
    unknown = self.run_context(
        "update", "--task-id", "TASK-404", "--progress", "Missing"
    )
    unknown_clear = self.run_context(
        "clear", "--task-id", "TASK-404"
    )
    invalid = self.run_context(
        "start", "--task-id", "bad task id", "--goal", "Invalid"
    )
    empty_update = self.run_context("update", "--task-id", "TASK-1")

    self.assertIn("already exists", duplicate.stderr)
    self.assertIn("not found", unknown.stderr)
    self.assertIn("not found", unknown_clear.stderr)
    self.assertIn("Task ID must use", invalid.stderr)
    self.assertIn("requires a changed field", empty_update.stderr)

def test_working_rejects_secret_without_echoing_it(self) -> None:
    fake_token = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"
    result = self.run_context(
        "start",
        "--task-id", "TASK-SECRET",
        "--goal", f"Rotate {fake_token}",
    )
    self.assertNotEqual(0, result.returncode)
    self.assertIn("possible GitHub token", result.stderr)
    self.assertNotIn("ABCDEFGHIJKLMNOPQRSTUVWXYZ", result.stderr)
```

- [ ] **Step 2: Run the working-memory tests and verify RED**

Run:

```bash
python3 -m unittest \
  Symfony/memory-bank/tests/test_context.py \
  -k working -v
```

Expected: argparse rejects the missing `start`, `update`, and `clear` commands.

- [ ] **Step 3: Add the regular working-task table**

Create this table during `connect()`:

```sql
CREATE TABLE IF NOT EXISTS working_tasks(
    task_id TEXT PRIMARY KEY,
    goal TEXT NOT NULL,
    progress TEXT NOT NULL,
    next_steps TEXT NOT NULL,
    files TEXT NOT NULL,
    sources TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
)
```

Store `next_steps`, `files`, and `sources` as JSON arrays. Add:

```python
from typing import Optional

TASK_ID_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$")

def validate_task_id(task_id: str) -> str:
    normalized = task_id.strip()
    if TASK_ID_PATTERN.fullmatch(normalized) is None:
        raise ContextError("Task ID must use letters, digits, '.', '_', '/', or '-'")
    return normalized
```

Extract the existing episode list normalization and secret scan into shared
helpers so working and episodic writes enforce the same boundary.

- [ ] **Step 4: Implement start, update, get, and clear**

Use parameterized SQL and transactions. `start` rejects an existing ID.
`update` replaces `progress` when supplied, appends unique repeated
`--next-step`, `--file`, and `--source` values, and rejects an empty update.
`clear` deletes exactly one known task.

Use these shared signatures:

```python
def normalize_values(label: str, values: list[str]) -> list[str]:
    normalized = [value.strip() for value in values]
    if any(not value for value in normalized):
        raise ContextError(f"{label} must not be empty")
    return normalized

def merge_unique(existing: list[str], additions: list[str]) -> list[str]:
    return list(dict.fromkeys((*existing, *additions)))

def reject_secrets(record_type: str, values: list[str]) -> None:
    candidate = "\n".join(values)
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(candidate):
            raise ContextError(
                f"possible {label} detected; {record_type} not stored"
            )
```

Register the commands with `argparse`, including `--json`, and add working-task
count to `status`.

- [ ] **Step 5: Run the full Symfony suite**

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -v
```

Expected: all tests pass, including task isolation and secret rejection.

- [ ] **Step 6: Commit working memory**

```bash
git add -- \
  Symfony/memory-bank/scripts/context.py \
  Symfony/memory-bank/tests/test_context.py
git commit -m "feat(symfony): add explicit working task memory"
```

### Task 3: Build A Layered Context Packet

**Files:**
- Modify: `Symfony/memory-bank/scripts/context.py`
- Test: `Symfony/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: `get_working_task()`, layer-aware document search, existing episode search
- Produces: `build_context_packet(connection: sqlite3.Connection, query: str, task_id: str, limit: int) -> dict[str, object]` and the `context` CLI command

- [ ] **Step 1: Write the failing context-packet test**

Create one source for each durable layer, start `BAUMAS-133`, index, and invoke:

```python
packet = self.run_context(
    "context",
    "password session invalidation",
    "--task-id", "BAUMAS-133",
    "--limit", "2",
    "--json",
)
self.assertEqual(0, packet.returncode, packet.stderr)
payload = json.loads(packet.stdout)
self.assertEqual("BAUMAS-133", payload["working"]["task_id"])
self.assertLessEqual(len(payload["procedural"]), 2)
self.assertLessEqual(len(payload["semantic"]), 2)
self.assertLessEqual(len(payload["episodic"]), 2)
self.assertTrue(all(item["layer"] == "procedural" for item in payload["procedural"]))
self.assertTrue(all(item["layer"] == "semantic" for item in payload["semantic"]))
self.assertTrue(all(item["layer"] == "episodic" for item in payload["episodic"]))
```

Also assert that an unknown task ID and a non-positive limit are rejected.

```python
unknown = self.run_context(
    "context", "password", "--task-id", "TASK-404", "--json"
)
invalid_limit = self.run_context(
    "context", "password", "--task-id", "BAUMAS-133",
    "--limit", "0", "--json",
)
self.assertIn("not found", unknown.stderr)
self.assertIn("--limit must be a positive integer", invalid_limit.stderr)
```

- [ ] **Step 2: Run the test and verify RED**

```bash
python3 -m unittest \
  Symfony/memory-bank/tests/test_context.py \
  -k context_packet -v
```

Expected: argparse rejects the `context` command.

- [ ] **Step 3: Implement filtered searches and packet assembly**

Allow document search to filter with:

```sql
WHERE documents MATCH ? AND layer = ?
ORDER BY bm25(documents), path
LIMIT ?
```

Tag local episode results with `"layer": "episodic"`. Build this JSON shape:

```json
{
  "query": "password session invalidation",
  "task_id": "BAUMAS-133",
  "working": {},
  "procedural": [],
  "semantic": [],
  "episodic": []
}
```

For episodic results, keep authoritative `CHANGELOG.md` matches before local
episodes and truncate the combined list to the per-layer limit. Default the
limit to three.

- [ ] **Step 4: Add human-readable layered output**

Print one `working`, `procedural`, `semantic`, and `episodic` heading. Do not
print empty durable sections. Keep paths and titles visible so users can verify
sources.

```python
print(f"working: {working['task_id']} — {working['goal']}")
for layer in ("procedural", "semantic", "episodic"):
    items = packet[layer]
    if not items:
        continue
    print(f"{layer}:")
    for item in items:
        label = item["path"] if "path" in item else f"episode {item['id']}"
        title = item["title"] if "title" in item else item["summary"]
        print(f"  {label} — {title}")
```

- [ ] **Step 5: Run the complete Symfony suite**

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -v
```

Expected: all tests pass and existing general search remains compatible.

- [ ] **Step 6: Commit context assembly**

```bash
git add -- \
  Symfony/memory-bank/scripts/context.py \
  Symfony/memory-bank/tests/test_context.py
git commit -m "feat(symfony): assemble layered context packets"
```

### Task 4: Complete Working Tasks Atomically

**Files:**
- Modify: `Symfony/memory-bank/scripts/context.py`
- Test: `Symfony/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: working-task rows and the existing episode schema
- Produces:
  - `insert_episode(connection: sqlite3.Connection, summary: str, outcome: str, files: list[str], verification: list[str], sources: list[str]) -> int`
  - `complete_working_task(connection: sqlite3.Connection, task_id: str, outcome: str, summary: Optional[str], files: list[str], verification: list[str], sources: list[str]) -> int`

- [ ] **Step 1: Write the failing successful-completion test**

Start and update a task, then invoke:

```python
completed = self.run_context(
    "complete",
    "--task-id", "BAUMAS-133",
    "--outcome", "Other sessions are invalidated.",
    "--file", "tests/Integration/GraphQL/ChangePasswordTest.php",
    "--verification", "ChangePasswordTest passed",
    "--json",
)
self.assertEqual(0, completed.returncode, completed.stderr)

status = json.loads(self.run_context("status", "--json").stdout)
self.assertEqual(0, status["working"])
self.assertEqual(1, status["episodes"])

episode = json.loads(
    self.run_context("search", "BAUMAS-133", "--json").stdout
)["episodes"][0]
self.assertEqual("Invalidate other password sessions.", episode["summary"])
self.assertIn("ChangePasswordTest passed", episode["verification"])
```

Use a goal containing the task ID so it remains searchable after completion.

- [ ] **Step 2: Write the failing rollback test**

After starting the task, open its test database and add:

```sql
CREATE TRIGGER fail_working_delete
BEFORE DELETE ON working_tasks
BEGIN
    SELECT RAISE(ABORT, 'simulated complete failure');
END;
```

Run `complete` and assert a non-zero result, one preserved working row, and zero
episodes. This proves the episode insert rolls back when deletion fails.

- [ ] **Step 3: Run both tests and verify RED**

```bash
python3 -m unittest \
  Symfony/memory-bank/tests/test_context.py \
  -k complete -v
```

Expected: argparse rejects the missing `complete` command.

- [ ] **Step 4: Separate episode insertion from transaction ownership**

Create:

```python
def insert_episode(
    connection: sqlite3.Connection,
    summary: str,
    outcome: str,
    files: list[str],
    verification: list[str],
    sources: list[str],
) -> int:
    summary = summary.strip()
    outcome = outcome.strip()
    if not summary or not outcome:
        raise ContextError("Episode summary and outcome must not be empty")
    files = normalize_values("Episode file", files)
    verification = normalize_values("Episode verification", verification)
    sources = normalize_values("Episode source", sources)
    reject_secrets(
        "episode",
        [summary, outcome, *files, *verification, *sources],
    )
    cursor = connection.execute(
        """
        INSERT INTO episodes(
            summary, outcome, files, verification, sources, created_at
        ) VALUES (?, ?, ?, ?, ?, ?)
        """,
        (
            summary,
            outcome,
            json.dumps(files, ensure_ascii=False),
            json.dumps(verification, ensure_ascii=False),
            json.dumps(sources, ensure_ascii=False),
            datetime.now(timezone.utc).isoformat(),
        ),
    )
    return int(cursor.lastrowid)
```

It validates and inserts but does not enter `with connection`. Keep
`record_episode()` as the compatibility wrapper that owns its normal
transaction.

- [ ] **Step 5: Implement atomic complete**

`complete_working_task()` must:

1. execute `BEGIN IMMEDIATE`;
2. load and validate the working task;
3. choose the optional summary or fall back to the task goal;
4. merge unique working and completion files/sources;
5. call `insert_episode()`;
6. delete the working row;
7. commit;
8. roll back on every exception.

Register `complete` with required `--task-id` and `--outcome`, optional
`--summary`, repeatable `--file`, `--verification`, and `--source`, and
`--json`.

- [ ] **Step 6: Run the complete Symfony suite**

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -v
```

Expected: success and rollback paths pass without regressing record or schema
migration behavior.

- [ ] **Step 7: Commit atomic completion**

```bash
git add -- \
  Symfony/memory-bank/scripts/context.py \
  Symfony/memory-bank/tests/test_context.py
git commit -m "feat(symfony): complete working tasks atomically"
```

### Task 5: Mirror The Verified Engine

**Files:**
- Modify: `Laravel/memory-bank/scripts/context.py`
- Modify: `Laravel/memory-bank/tests/test_context.py`
- Modify: `PHP Core/memory-bank/scripts/context.py`
- Modify: `PHP Core/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: the verified Symfony script and tests
- Produces: byte-identical four-layer behavior in all three accelerators

- [ ] **Step 1: Copy the proven canonical files**

```bash
cp Symfony/memory-bank/scripts/context.py Laravel/memory-bank/scripts/context.py
cp Symfony/memory-bank/scripts/context.py "PHP Core/memory-bank/scripts/context.py"
cp Symfony/memory-bank/tests/test_context.py Laravel/memory-bank/tests/test_context.py
cp Symfony/memory-bank/tests/test_context.py "PHP Core/memory-bank/tests/test_context.py"
```

- [ ] **Step 2: Verify byte parity**

```bash
cmp Symfony/memory-bank/scripts/context.py Laravel/memory-bank/scripts/context.py
cmp Symfony/memory-bank/scripts/context.py "PHP Core/memory-bank/scripts/context.py"
cmp Symfony/memory-bank/tests/test_context.py Laravel/memory-bank/tests/test_context.py
cmp Symfony/memory-bank/tests/test_context.py "PHP Core/memory-bank/tests/test_context.py"
```

Expected: every command exits zero.

- [ ] **Step 3: Run all three suites**

```bash
for edition in Laravel "PHP Core" Symfony; do
  python3 -m unittest discover -s "$edition/memory-bank/tests" -v
done
```

Expected: all validator and Context Engine tests pass in every edition.

- [ ] **Step 4: Commit the mirrors**

```bash
git add -- \
  Laravel/memory-bank/scripts/context.py \
  Laravel/memory-bank/tests/test_context.py \
  "PHP Core/memory-bank/scripts/context.py" \
  "PHP Core/memory-bank/tests/test_context.py"
git commit -m "feat: mirror four-layer context memory"
```

### Task 6: Integrate The Four-Layer Workflow And Verify A Real Project

**Files:**
- Modify: `README.md`
- Modify: `Laravel/AGENTS.md`
- Modify: `PHP Core/AGENTS.md`
- Modify: `Symfony/AGENTS.md`
- Modify: `Laravel/memory-bank/README.md`
- Modify: `PHP Core/memory-bank/README.md`
- Modify: `Symfony/memory-bank/README.md`
- Modify: `Laravel/.agents/skills/memory-bank/SKILL.md`
- Modify: `Laravel/.claude/skills/memory-bank/SKILL.md`
- Modify: `Laravel/.cursor/skills/memory-bank/SKILL.md`
- Modify: `PHP Core/.agents/skills/memory-bank/SKILL.md`
- Modify: `PHP Core/.claude/skills/memory-bank/SKILL.md`
- Modify: `PHP Core/.cursor/skills/memory-bank/SKILL.md`
- Modify: `Symfony/.agents/skills/memory-bank/SKILL.md`
- Modify: `Symfony/.claude/skills/memory-bank/SKILL.md`
- Modify: `Symfony/.cursor/skills/memory-bank/SKILL.md`
- Modify: `Laravel/CHANGELOG.md`
- Modify: `PHP Core/CHANGELOG.md`
- Modify: `Symfony/CHANGELOG.md`

**Interfaces:**
- Consumes: all verified CLI commands
- Produces: discoverable workflow guidance and current changelog entries

- [ ] **Step 1: Expand the root README**

Add a `Local Context Engine` section after the existing cross-edition
memory-bank overview. Explain the four logical layers in plain language, show
the `start → update → context → complete` lifecycle, state that repository
sources remain authoritative, mention ignored SQLite storage and secret
rejection, and report the Bauherrenmappe proof without claiming automatic
per-request injection, embeddings, MCP, or LangGraph.

- [ ] **Step 2: Update the three memory-bank READMEs**

Document exact commands:

```bash
python3 memory-bank/scripts/context.py index
python3 memory-bank/scripts/context.py start \
  --task-id BAUMAS-133 \
  --goal "Invalidate other password-change sessions"
python3 memory-bank/scripts/context.py update \
  --task-id BAUMAS-133 \
  --progress "Two-session regression passes" \
  --next-step "Verify the old remember-me cookie"
python3 memory-bank/scripts/context.py context \
  "password session invalidation" \
  --task-id BAUMAS-133
python3 memory-bank/scripts/context.py complete \
  --task-id BAUMAS-133 \
  --outcome "Other sessions and stale remember-me cookies are invalidated" \
  --verification "ChangePasswordTest passed"
```

Explain sources, lifecycle, `clear`, general search/record compatibility,
database locality, and deletion implications for local working and episode
data.

- [ ] **Step 3: Update policies and all nine skill mirrors**

Replace the old “record an episode after completion” guidance with:

1. use a ticket ID, branch name, or descriptive slug;
2. `start` before non-trivial work;
3. `context` before making decisions;
4. `update` only with sanitized progress;
5. `complete` after verification;
6. never capture raw conversation or logs.

Keep framework wording local to each edition and keep the three tool mirrors
within an edition equivalent.

- [ ] **Step 4: Update all three changelogs**

Extend the unreleased Context Engine entry with four-layer classification,
explicit task IDs, bounded context packets, and atomic working-to-episode
completion.

- [ ] **Step 5: Run final repository verification**

```bash
for edition in Laravel "PHP Core" Symfony; do
  python3 -m unittest discover -s "$edition/memory-bank/tests" -v
  python3 "$edition/memory-bank/scripts/validate.py"
done
python3 -m py_compile \
  Laravel/memory-bank/scripts/context.py \
  "PHP Core/memory-bank/scripts/context.py" \
  Symfony/memory-bank/scripts/context.py
git diff --check
```

Expected: every suite and validator passes, compilation succeeds, and no
whitespace errors are reported.

- [ ] **Step 6: Run the Bauherrenmappe black-box scenario**

Use a `mktemp -d` database outside the project. Against
`/home/aliaksei/Desktop/bauherrenmappe`:

1. index and confirm procedural, semantic, and episodic counts;
2. start `BAUMAS-133`;
3. update it with a real resolver and test path;
4. build a context packet for `PdoSessionHandler`;
5. complete it with sanitized verification;
6. search the resulting episode;
7. confirm the Bauherrenmappe `git status --porcelain=v1` is unchanged;
8. delete only the validated temporary directory.

- [ ] **Step 7: Commit documentation and workflow integration**

Stage only the files listed in this task:

```bash
git add -- \
  README.md \
  Laravel/AGENTS.md "PHP Core/AGENTS.md" Symfony/AGENTS.md \
  Laravel/memory-bank/README.md \
  "PHP Core/memory-bank/README.md" \
  Symfony/memory-bank/README.md \
  Laravel/.agents/skills/memory-bank/SKILL.md \
  Laravel/.claude/skills/memory-bank/SKILL.md \
  Laravel/.cursor/skills/memory-bank/SKILL.md \
  "PHP Core/.agents/skills/memory-bank/SKILL.md" \
  "PHP Core/.claude/skills/memory-bank/SKILL.md" \
  "PHP Core/.cursor/skills/memory-bank/SKILL.md" \
  Symfony/.agents/skills/memory-bank/SKILL.md \
  Symfony/.claude/skills/memory-bank/SKILL.md \
  Symfony/.cursor/skills/memory-bank/SKILL.md \
  Laravel/CHANGELOG.md "PHP Core/CHANGELOG.md" Symfony/CHANGELOG.md
git commit -m "docs: explain four-layer context workflow"
```
