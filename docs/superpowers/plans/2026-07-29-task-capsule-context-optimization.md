# Task Capsule Context Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a deterministic, OpenGSD-inspired Task Capsule that reduces context transferred to agents while preserving the existing local four-layer Context Engine and command lifecycle.

**Architecture:** Extend the existing `context` operation instead of adding another command or store. A capsule is assembled on demand from a concise sanitized query derived from the current request, optional Working state, and bounded FTS5 results; policy then passes that capsule rather than the parent conversation at complex phase boundaries.

**Tech Stack:** Python 3.9+ standard library, SQLite FTS5, `unittest`, Markdown agent policies, Git, DDEV/PHPUnit for the Bauherrenmappe pressure check.

## Global Constraints

- Keep one repository-local ignored SQLite database with Working, Procedural, Semantic, and Episodic layers.
- Do not install or wrap `gsd-core` or `gsd-pi`.
- Do not add `.planning/`, another database, another task lifecycle, embeddings, vector search, MCP, a tokenizer dependency, or a central service.
- Keep `memory` and `checkpoint` argument-free and preserve explicit `complete`.
- Keep existing `context.py` commands and explicit lifecycle arguments compatible.
- The serialized capsule must not exceed 8,000 Unicode characters.
- A capsule contains at most two Procedural, three Semantic, and one Episodic result.
- Never store or pass parent conversation history, raw diffs, logs, prompts, responses, reasoning, secrets, ignored-file contents, or binary contents.
- Repository policy, code, configuration, tests, and specifications remain authoritative.
- Keep `context.py`, `test_context.py`, and `test_memory.py` byte-identical across Laravel, Symfony, and PHP Core after their common changes.
- Preserve all unrelated work and leave `.langgraph_api/` and `ai_infrastructure_langgraph.egg-info/` unstaged.

## File Map

- `Symfony/memory-bank/scripts/context.py`: canonical implementation for capsule retrieval, compaction, and degraded-index handling.
- `Symfony/memory-bank/tests/test_context.py`: canonical executable contract for capsule behavior.
- `Laravel/memory-bank/scripts/context.py`, `PHP Core/memory-bank/scripts/context.py`: byte-identical mirrors.
- `Laravel/memory-bank/tests/test_context.py`, `PHP Core/memory-bank/tests/test_context.py`: byte-identical mirrors.
- `Symfony/memory-bank/tests/test_memory.py` and mirrors: policy, skill-flow, and documentation contract.
- `Symfony/AGENTS.md`, `Laravel/AGENTS.md`, `PHP Core/AGENTS.md`: automatic hybrid-routing policy.
- Each edition's `.agents/skills/SKILL FLOW.md`, `.claude/skills/SKILL FLOW.md`, and `.cursor/skills/SKILL FLOW.md`: phase handoff contract.
- Each edition's `README.md`, `memory-bank/README.md`, and `CHANGELOG.md`: stack-local user documentation.
- `README_EN.md`, `README_RU.md`: repository-level user documentation.
- `docs/superpowers/reports/2026-07-29-task-capsule-bauherrenmappe.md`: measured real-project evidence.

---

### Task 1: Build the bounded layered capsule contract in the canonical Symfony copy

**Files:**
- Modify: `Symfony/memory-bank/tests/test_context.py`
- Modify: `Symfony/memory-bank/scripts/context.py`

**Interfaces:**
- Consumes: existing `search_documents()`, `search_episodes()`, `get_working_task()`, and the `context` CLI operation.
- Produces:
  - `CAPSULE_LAYER_LIMITS: dict[str, int]`
  - `find_working_task(connection: sqlite3.Connection, task_id: Optional[str]) -> Optional[dict[str, object]]`
  - `build_capsule_query(query: str, working: Optional[dict[str, object]]) -> str`
  - `deduplicate_context_items(items: list[dict[str, object]]) -> list[dict[str, object]]`
  - `build_context_packet(connection: sqlite3.Connection, query: str, task_id: Optional[str], limit: int, include_retrieval: bool = True, warnings: Optional[list[str]] = None) -> dict[str, object]`
- Compatibility: `context QUERY --task-id TASK --limit N --json` remains valid; `--task-id` becomes optional for request-only retrieval.

- [ ] **Step 1: Add failing request-only and layer-limit tests**

Append these methods to `ContextEngineTest` near the existing context-packet test:

```python
    def test_context_builds_request_only_capsule_with_layer_limits(self) -> None:
        procedural_sources = {
            "AGENTS.md": "# Policy\n\nThe capsule boundary is mandatory.\n",
            "CLAUDE.md": "# Claude\n\nThe capsule boundary guides work.\n",
            ".agents/skills/alpha/SKILL.md": (
                "# Alpha\n\nThe capsule boundary applies to alpha.\n"
            ),
        }
        for relative_path, content in procedural_sources.items():
            path = self.repository / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")

        semantic_sources = {
            "README.md": (
                "# Project\n\nThe capsule boundary describes the project.\n\n"
                + ("Background material without the search terms. " * 80)
                + "\nFULL_DOCUMENT_SENTINEL\n"
            ),
            "specs/one.md": "# One\n\nThe capsule boundary protects one.\n",
            "specs/two.md": "# Two\n\nThe capsule boundary protects two.\n",
            "specs/three.md": "# Three\n\nThe capsule boundary protects three.\n",
        }
        for relative_path, content in semantic_sources.items():
            (self.repository / relative_path).write_text(content, encoding="utf-8")

        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changes\n\nThe capsule boundary shipped.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)

        packet = self.run_context(
            "context",
            "capsule boundary",
            "--limit",
            "20",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertIsNone(payload["task_id"])
        self.assertIsNone(payload["working"])
        self.assertLessEqual(len(payload["procedural"]), 2)
        self.assertLessEqual(len(payload["semantic"]), 3)
        self.assertLessEqual(len(payload["episodic"]), 1)
        self.assertIn("Working task unavailable: task ID was not supplied", payload["warnings"])
        self.assertNotIn("FULL_DOCUMENT_SENTINEL", packet.stdout)

    def test_context_uses_working_terms_and_deduplicates_paths(self) -> None:
        self.repository.joinpath("README.md").write_text(
            "# Cobalt\n\nThe cobalt invariant belongs to the account workflow.\n",
            encoding="utf-8",
        )
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "BAUMAS-133",
                "--goal",
                "Verify the cobalt invariant.",
            ).returncode,
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)

        packet = self.run_context(
            "context",
            "current phase",
            "--task-id",
            "BAUMAS-133",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertEqual("BAUMAS-133", payload["task_id"])
        self.assertEqual("BAUMAS-133", payload["working"]["task_id"])
        self.assertEqual(
            ["README.md"],
            [item["path"] for item in payload["semantic"]],
        )
        document = {
            "path": "README.md",
            "layer": "semantic",
            "kind": "overview",
            "title": "Project",
            "snippet": "Cobalt invariant.",
        }
        episode = {
            "id": 7,
            "layer": "episodic",
            "summary": "Prior work",
        }
        self.assertEqual(
            [document, episode],
            CONTEXT.deduplicate_context_items(
                [document, dict(document), episode, dict(episode)]
            ),
        )

    def test_context_rejects_secret_query_without_echoing_it(self) -> None:
        secret = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"

        packet = self.run_context("context", secret, "--json")

        self.assertNotEqual(0, packet.returncode)
        self.assertIn("possible GitHub token", packet.stderr)
        self.assertNotIn("ABCDEFGHIJKLMNOPQRSTUVWXYZ", packet.stderr)
```

- [ ] **Step 2: Update the old unknown-task expectation**

In `test_context_packet_retrieves_each_layer_for_working_task`, change the
Episodic assertions to the new one-item capsule maximum:

```python
        self.assertEqual(1, len(payload["episodic"]))
        self.assertEqual("CHANGELOG.md", payload["episodic"][0]["path"])
```

Remove the old assertion that expected a second local episode. Regular
`search --layer episodic` continues to cover local-episode retrieval.

Then replace the unknown-task error assertion with the approved ephemeral
behavior:

```python
        unknown = self.run_context(
            "context",
            "password",
            "--task-id",
            "TASK-404",
            "--json",
        )
        invalid_limit = self.run_context(
            "context",
            "password",
            "--task-id",
            "BAUMAS-133",
            "--limit",
            "0",
            "--json",
        )
        self.assertEqual(0, unknown.returncode, unknown.stderr)
        unknown_payload = json.loads(unknown.stdout)
        self.assertIsNone(unknown_payload["working"])
        self.assertIn("Working task not found: TASK-404", unknown_payload["warnings"])
        self.assertIn("--limit must be a positive integer", invalid_limit.stderr)
```

- [ ] **Step 3: Run the focused tests and verify they fail**

Run:

```bash
cd Symfony
python3 -m unittest discover -s memory-bank/tests -p 'test_context.py' -v
```

Expected: FAIL because `--task-id` is still required, layer-specific capsule
limits do not exist, duplicate paths are returned, and unknown tasks still
abort.

- [ ] **Step 4: Add the minimal retrieval helpers**

In `Symfony/memory-bank/scripts/context.py`, add these constants beside
`DOCUMENT_LAYERS`:

```python
CAPSULE_LAYER_LIMITS = {
    "procedural": 2,
    "semantic": 3,
    "episodic": 1,
}
CAPSULE_QUERY_TOKEN_LIMIT = 32
```

Refactor Working lookup so explicit commands still raise while capsule lookup
can be optional:

```python
def find_working_task(
    connection: sqlite3.Connection, task_id: Optional[str]
) -> Optional[dict[str, object]]:
    if task_id is None:
        return None
    task_id = validate_task_id(task_id)
    row = connection.execute(
        """
        SELECT task_id, goal, progress, next_steps, files, sources,
               created_at, updated_at
        FROM working_tasks
        WHERE task_id = ?
        """,
        (task_id,),
    ).fetchone()
    if row is None:
        return None
    return {
        "task_id": row["task_id"],
        "goal": row["goal"],
        "progress": row["progress"],
        "next_steps": json.loads(row["next_steps"]),
        "files": json.loads(row["files"]),
        "sources": json.loads(row["sources"]),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def get_working_task(
    connection: sqlite3.Connection, task_id: str
) -> dict[str, object]:
    task = find_working_task(connection, task_id)
    if task is None:
        raise ContextError(f"Working task not found: {validate_task_id(task_id)}")
    return task
```

Add deterministic query enrichment and source deduplication:

```python
def build_capsule_query(
    query: str, working: Optional[dict[str, object]]
) -> str:
    values = [query]
    if working is not None:
        values.extend(
            [
                str(working["goal"]),
                str(working["progress"]),
                *[str(item) for item in working["next_steps"]],
                *[Path(str(item)).stem for item in working["files"]],
            ]
        )
    tokens: list[str] = []
    for token in re.findall(r"\w+", " ".join(values), flags=re.UNICODE):
        normalized = token.casefold()
        if normalized not in {item.casefold() for item in tokens}:
            tokens.append(token)
        if len(tokens) == CAPSULE_QUERY_TOKEN_LIMIT:
            break
    if not tokens:
        raise ContextError("Search query must contain a word")
    return " ".join(tokens)


def deduplicate_context_items(
    items: list[dict[str, object]],
) -> list[dict[str, object]]:
    unique: list[dict[str, object]] = []
    seen: set[tuple[str, object]] = set()
    for item in items:
        key = (
            ("path", item["path"])
            if "path" in item
            else ("episode", item["id"])
        )
        if key in seen:
            continue
        seen.add(key)
        unique.append(item)
    return unique
```

Use a `seen_tokens: set[str]` local in the final implementation instead of
rebuilding a set inside the loop; keep the behavior shown above.

- [ ] **Step 5: Replace `build_context_packet` with the capsule-aware form**

Implement this exact interface and field shape:

```python
def build_context_packet(
    connection: sqlite3.Connection,
    query: str,
    task_id: Optional[str],
    limit: int,
    include_retrieval: bool = True,
    warnings: Optional[list[str]] = None,
) -> dict[str, object]:
    if limit < 1:
        raise ContextError("--limit must be a positive integer")
    reject_secrets("Task Capsule", [query])

    capsule_warnings = list(warnings or [])
    working = find_working_task(connection, task_id)
    if task_id is None:
        capsule_warnings.append(
            "Working task unavailable: task ID was not supplied"
        )
    elif working is None:
        capsule_warnings.append(f"Working task not found: {validate_task_id(task_id)}")

    packet: dict[str, object] = {
        "query": query,
        "task_id": task_id,
        "working": working,
        "procedural": [],
        "semantic": [],
        "episodic": [],
        "warnings": capsule_warnings,
        "omitted": {"working_files": 0},
    }
    if not include_retrieval:
        return packet

    retrieval_query = build_capsule_query(query, working)
    procedural_limit = min(limit, CAPSULE_LAYER_LIMITS["procedural"])
    semantic_limit = min(limit, CAPSULE_LAYER_LIMITS["semantic"])
    episodic_limit = min(limit, CAPSULE_LAYER_LIMITS["episodic"])
    packet["procedural"] = deduplicate_context_items(
        search_documents(connection, retrieval_query, procedural_limit, "procedural")
    )[:procedural_limit]
    packet["semantic"] = deduplicate_context_items(
        search_documents(connection, retrieval_query, semantic_limit, "semantic")
    )[:semantic_limit]
    packet["episodic"] = deduplicate_context_items(
        search_documents(connection, retrieval_query, episodic_limit, "episodic")
        + search_episodes(connection, retrieval_query, episodic_limit)
    )[:episodic_limit]
    return packet
```

Make `context.add_argument("--task-id")` optional and keep `--limit` as an
upper bound.

- [ ] **Step 6: Run the focused tests and verify they pass**

Run the command from Step 3.

Expected: PASS.

- [ ] **Step 7: Run the complete canonical context test file**

Run:

```bash
cd Symfony
python3 -m unittest discover -s memory-bank/tests -p 'test_context.py' -v
```

Expected: all context-engine tests PASS.

- [ ] **Step 8: Commit the canonical retrieval contract**

```bash
git add Symfony/memory-bank/scripts/context.py \
  Symfony/memory-bank/tests/test_context.py
git commit -m "feat: add bounded task capsule retrieval"
```

---

### Task 2: Enforce the character budget and degraded-index fallback

**Files:**
- Modify: `Symfony/memory-bank/tests/test_context.py`
- Modify: `Symfony/memory-bank/scripts/context.py`

**Interfaces:**
- Consumes: Task 1's `build_context_packet()` field shape.
- Produces:
  - `CAPSULE_CHARACTER_LIMIT = 8000`
  - `serialize_capsule(capsule: dict[str, object]) -> str`
  - `capsule_character_count(capsule: dict[str, object]) -> int`
  - `enforce_capsule_budget(capsule: dict[str, object]) -> dict[str, object]`
- CLI behavior: `context` refreshes the source index; a refresh failure returns
  request/Working data with empty long-lived layers and a safe warning.

- [ ] **Step 1: Add failing budget and progressive-disclosure tests**

Add these tests to `ContextEngineTest`:

```python
    def test_context_capsule_stays_within_character_budget(self) -> None:
        self.repository.joinpath("README.md").write_text(
            "# Project\n\nCapsule budget knowledge.\n",
            encoding="utf-8",
        )
        long_progress = "verified progress " * 700
        long_files = [f"src/Feature/File{index:03d}.php" for index in range(200)]
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "CAPSULE-1",
                "--goal",
                "Keep the capsule budget bounded.",
                *[
                    argument
                    for file_path in long_files
                    for argument in ("--file", file_path)
                ],
            ).returncode,
        )
        self.assertEqual(
            0,
            self.run_context(
                "update",
                "--task-id",
                "CAPSULE-1",
                "--progress",
                long_progress,
            ).returncode,
        )

        packet = self.run_context(
            "context",
            "capsule budget",
            "--task-id",
            "CAPSULE-1",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        self.assertLessEqual(
            len(packet.stdout.rstrip("\n")),
            CONTEXT.CAPSULE_CHARACTER_LIMIT,
        )
        payload = json.loads(packet.stdout)
        self.assertGreater(payload["omitted"]["working_files"], 0)
        self.assertEqual(
            "Keep the capsule budget bounded.",
            payload["working"]["goal"],
        )

    def test_capsule_budget_drops_optional_layers_before_working(self) -> None:
        capsule = {
            "query": "capsule",
            "task_id": "CAPSULE-2",
            "working": {
                "task_id": "CAPSULE-2",
                "goal": "Preserve the goal.",
                "progress": "p" * 300,
                "next_steps": ["Run verification."],
                "files": ["src/Required.php"],
                "sources": [],
                "created_at": "2026-07-29T00:00:00+00:00",
                "updated_at": "2026-07-29T00:00:00+00:00",
            },
            "procedural": [
                {
                    "path": "AGENTS.md",
                    "layer": "procedural",
                    "kind": "policy",
                    "title": "Policy",
                    "snippet": "Mandatory capsule policy.",
                }
            ],
            "semantic": [
                {
                    "path": f"specs/{index}.md",
                    "layer": "semantic",
                    "kind": "spec",
                    "title": str(index),
                    "snippet": "s" * 250,
                }
                for index in range(3)
            ],
            "episodic": [
                {
                    "id": 1,
                    "layer": "episodic",
                    "summary": "Prior work",
                    "outcome": "e" * 300,
                    "files": [],
                    "verification": ["Verified"],
                    "sources": [],
                    "created_at": "2026-07-29T00:00:00+00:00",
                }
            ],
            "warnings": [],
            "omitted": {"working_files": 0},
        }

        with mock.patch.object(CONTEXT, "CAPSULE_CHARACTER_LIMIT", 1250):
            compacted = CONTEXT.enforce_capsule_budget(capsule)

        self.assertEqual([], compacted["episodic"])
        self.assertEqual("Preserve the goal.", compacted["working"]["goal"])
        self.assertEqual("AGENTS.md", compacted["procedural"][0]["path"])
        self.assertLessEqual(CONTEXT.capsule_character_count(compacted), 1250)

    def test_capsule_rejects_mandatory_content_larger_than_budget(self) -> None:
        capsule = {
            "query": "capsule",
            "task_id": None,
            "working": None,
            "procedural": [
                {
                    "path": "AGENTS.md",
                    "layer": "procedural",
                    "kind": "policy",
                    "title": "Policy",
                    "snippet": "mandatory " * 100,
                }
            ],
            "semantic": [],
            "episodic": [],
            "warnings": [],
            "omitted": {"working_files": 0},
        }

        with mock.patch.object(CONTEXT, "CAPSULE_CHARACTER_LIMIT", 200):
            with self.assertRaisesRegex(
                CONTEXT.ContextError,
                "mandatory Task Capsule content exceeds 200 characters",
            ):
                CONTEXT.enforce_capsule_budget(capsule)
```

- [ ] **Step 2: Add the failing degraded-index test**

```python
    def test_context_index_failure_returns_working_only_without_stale_sources(
        self,
    ) -> None:
        self.repository.joinpath("README.md").write_text(
            "# Project\n\nThe celadon source was previously indexed.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "CAPSULE-3",
                "--goal",
                "Inspect celadon behavior.",
            ).returncode,
        )
        self.repository.joinpath("docs").mkdir(exist_ok=True)
        self.repository.joinpath("docs/broken.md").write_bytes(b"\xff")

        packet = self.run_context(
            "context",
            "celadon",
            "--task-id",
            "CAPSULE-3",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertEqual("CAPSULE-3", payload["working"]["task_id"])
        self.assertEqual([], payload["procedural"])
        self.assertEqual([], payload["semantic"])
        self.assertEqual([], payload["episodic"])
        self.assertIn(
            "Index refresh failed: Source document is not valid UTF-8: docs/broken.md",
            payload["warnings"],
        )
        self.assertNotIn("celadon source", packet.stdout)
        self.assertNotIn("Traceback", packet.stderr)
```

- [ ] **Step 3: Run the new tests and verify they fail**

Run:

```bash
cd Symfony
python3 -m unittest discover -s memory-bank/tests -p 'test_context.py' -v
```

Expected: FAIL because the budget helpers do not exist, `context` does not
refresh the index, and output has no deterministic compaction.

- [ ] **Step 4: Implement deterministic serialization and compaction**

Add:

```python
CAPSULE_CHARACTER_LIMIT = 8000


def serialize_capsule(capsule: dict[str, object]) -> str:
    return json.dumps(
        capsule,
        ensure_ascii=False,
        separators=(",", ":"),
    )


def capsule_character_count(capsule: dict[str, object]) -> int:
    return len(serialize_capsule(capsule))


def enforce_capsule_budget(
    capsule: dict[str, object],
) -> dict[str, object]:
    compacted = json.loads(serialize_capsule(capsule))
    omitted = compacted["omitted"]
    working = compacted["working"]

    while capsule_character_count(compacted) > CAPSULE_CHARACTER_LIMIT:
        if compacted["episodic"]:
            compacted["episodic"].pop()
            continue
        if compacted["semantic"]:
            compacted["semantic"].pop()
            continue
        if working is not None and len(working["files"]) > 1:
            working["files"].pop()
            omitted["working_files"] += 1
            continue
        if working is not None and working["progress"]:
            excess = capsule_character_count(compacted) - CAPSULE_CHARACTER_LIMIT
            keep = max(0, len(working["progress"]) - excess - 1)
            working["progress"] = (
                f"{working['progress'][:keep]}…" if keep else ""
            )
            continue
        raise ContextError(
            "mandatory Task Capsule content exceeds "
            f"{CAPSULE_CHARACTER_LIMIT} characters"
        )

    return compacted
```

Do not remove the goal, next step, Procedural results, request query, or source
identity to make the packet fit.

- [ ] **Step 5: Refresh the index inside the `context` operation**

In the `context` branch of `main()`:

```python
            if arguments.command == "context":
                warnings: list[str] = []
                include_retrieval = True
                try:
                    index_repository(connection, repository)
                except (ContextError, OSError, sqlite3.Error) as error:
                    include_retrieval = False
                    warnings.append(f"Index refresh failed: {error}")
                result = build_context_packet(
                    connection,
                    arguments.query,
                    arguments.task_id,
                    arguments.limit,
                    include_retrieval=include_retrieval,
                    warnings=warnings,
                )
                result = enforce_capsule_budget(result)
                if arguments.json:
                    print(serialize_capsule(result))
                else:
                    working = result["working"]
                    if working is None:
                        print("working: unavailable")
                    else:
                        print(
                            f"working: {working['task_id']} — {working['goal']}"
                        )
                    for warning in result["warnings"]:
                        print(f"warning: {warning}")
                    for layer in ("procedural", "semantic", "episodic"):
                        items = result[layer]
                        if not items:
                            continue
                        print(f"{layer}:")
                        for item in items:
                            label = (
                                item["path"]
                                if "path" in item
                                else f"episode {item['id']}"
                            )
                            title = (
                                item["title"]
                                if "title" in item
                                else item["summary"]
                            )
                            print(f"  {label} — {title}")
                return 0
```

Keep the successful Working task and previous database state intact when
indexing fails.

- [ ] **Step 6: Run the new focused tests and verify they pass**

Run the command from Step 3.

Expected: PASS.

- [ ] **Step 7: Run every canonical memory-bank test**

```bash
cd Symfony
python3 -m unittest discover -s memory-bank/tests -v
python3 memory-bank/scripts/validate.py
```

Expected: all tests PASS and validation prints
`Memory bank validation passed.`

- [ ] **Step 8: Commit the budget and fallback**

```bash
git add Symfony/memory-bank/scripts/context.py \
  Symfony/memory-bank/tests/test_context.py
git commit -m "feat: enforce task capsule budget"
```

---

### Task 3: Mirror the executable capsule implementation across editions

**Files:**
- Modify: `Laravel/memory-bank/scripts/context.py`
- Modify: `PHP Core/memory-bank/scripts/context.py`
- Modify: `Laravel/memory-bank/tests/test_context.py`
- Modify: `PHP Core/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: the complete canonical files from Tasks 1 and 2.
- Produces: byte-identical Context Engine implementation and tests in all three editions.

- [ ] **Step 1: Copy the canonical implementation mechanically**

```bash
cp Symfony/memory-bank/scripts/context.py \
  Laravel/memory-bank/scripts/context.py
cp Symfony/memory-bank/scripts/context.py \
  'PHP Core/memory-bank/scripts/context.py'
cp Symfony/memory-bank/tests/test_context.py \
  Laravel/memory-bank/tests/test_context.py
cp Symfony/memory-bank/tests/test_context.py \
  'PHP Core/memory-bank/tests/test_context.py'
```

- [ ] **Step 2: Prove byte parity**

```bash
cmp Symfony/memory-bank/scripts/context.py \
  Laravel/memory-bank/scripts/context.py
cmp Symfony/memory-bank/scripts/context.py \
  'PHP Core/memory-bank/scripts/context.py'
cmp Symfony/memory-bank/tests/test_context.py \
  Laravel/memory-bank/tests/test_context.py
cmp Symfony/memory-bank/tests/test_context.py \
  'PHP Core/memory-bank/tests/test_context.py'
```

Expected: every command exits `0` with no output.

- [ ] **Step 3: Run the full suite and validator in every edition**

```bash
(cd Symfony && python3 -m unittest discover -s memory-bank/tests -q)
(cd Laravel && python3 -m unittest discover -s memory-bank/tests -q)
(cd 'PHP Core' && python3 -m unittest discover -s memory-bank/tests -q)
(cd Symfony && python3 memory-bank/scripts/validate.py)
(cd Laravel && python3 memory-bank/scripts/validate.py)
(cd 'PHP Core' && python3 memory-bank/scripts/validate.py)
```

Expected: all three suites report `OK`; all three validators pass.

- [ ] **Step 4: Commit the mirrors**

```bash
git add Laravel/memory-bank/scripts/context.py \
  Laravel/memory-bank/tests/test_context.py \
  'PHP Core/memory-bank/scripts/context.py' \
  'PHP Core/memory-bank/tests/test_context.py'
git commit -m "feat: mirror task capsule context"
```

---

### Task 4: Define automatic hybrid routing in policy and documentation

**Files:**
- Modify: `Symfony/memory-bank/tests/test_memory.py`
- Modify: `Laravel/memory-bank/tests/test_memory.py`
- Modify: `PHP Core/memory-bank/tests/test_memory.py`
- Modify: `Symfony/AGENTS.md`
- Modify: `Laravel/AGENTS.md`
- Modify: `PHP Core/AGENTS.md`
- Modify: `Symfony/.agents/skills/SKILL FLOW.md`
- Modify: `Symfony/.claude/skills/SKILL FLOW.md`
- Modify: `Symfony/.cursor/skills/SKILL FLOW.md`
- Modify: `Laravel/.agents/skills/SKILL FLOW.md`
- Modify: `Laravel/.claude/skills/SKILL FLOW.md`
- Modify: `Laravel/.cursor/skills/SKILL FLOW.md`
- Modify: `PHP Core/.agents/skills/SKILL FLOW.md`
- Modify: `PHP Core/.claude/skills/SKILL FLOW.md`
- Modify: `PHP Core/.cursor/skills/SKILL FLOW.md`
- Modify: `Symfony/README.md`
- Modify: `Laravel/README.md`
- Modify: `PHP Core/README.md`
- Modify: `Symfony/memory-bank/README.md`
- Modify: `Laravel/memory-bank/README.md`
- Modify: `PHP Core/memory-bank/README.md`
- Modify: `Symfony/CHANGELOG.md`
- Modify: `Laravel/CHANGELOG.md`
- Modify: `PHP Core/CHANGELOG.md`
- Modify: `README_EN.md`
- Modify: `README_RU.md`

**Interfaces:**
- Consumes: capsule JSON contract from Tasks 1–3.
- Produces: one policy-level phase gate; no new skill, command, wrapper, or database table.

- [ ] **Step 1: Add failing policy and documentation contract tests**

Add this test to the canonical `Symfony/memory-bank/tests/test_memory.py`:

```python
    def test_policy_and_docs_define_hybrid_task_capsules(self) -> None:
        policy = REPOSITORY_ROOT.joinpath("AGENTS.md").read_text(encoding="utf-8")
        required_policy = (
            "Task Capsule",
            "8,000 Unicode characters",
            "two Procedural",
            "three Semantic",
            "one Episodic",
            "MUST NOT pass the parent conversation",
            "start of a complex request",
            "research to planning",
            "planning to implementation",
            "implementation to independent verification",
            "simple task",
            "explicit `complete`",
        )
        for text in required_policy:
            with self.subTest(policy=text):
                self.assertIn(text, policy)

        for tool in (".agents", ".claude", ".cursor"):
            flow = REPOSITORY_ROOT.joinpath(
                tool, "skills", "SKILL FLOW.md"
            ).read_text(encoding="utf-8")
            with self.subTest(tool=tool):
                self.assertIn("## Task Capsule Handoff", flow)
                self.assertIn("work completed", flow)
                self.assertIn("verification evidence", flow)
                self.assertIn("unresolved blockers", flow)
                self.assertIn("must not preload every cited source", flow)

        documents = (
            REPOSITORY_ROOT / "README.md",
            REPOSITORY_ROOT / "memory-bank/README.md",
            REPOSITORY_ROOT.parent / "README_EN.md",
            REPOSITORY_ROOT.parent / "README_RU.md",
        )
        for path in documents:
            text = path.read_text(encoding="utf-8")
            with self.subTest(document=path):
                self.assertIn("Task Capsule", text)
                self.assertIn("8", text)
                self.assertIn("Procedural", text)
                self.assertIn("Semantic", text)
                self.assertIn("Episodic", text)
```

Copy the changed test file byte-for-byte to Laravel and PHP Core.

- [ ] **Step 2: Run the new contract test and verify it fails in all editions**

```bash
(cd Symfony && python3 -m unittest discover -s memory-bank/tests -p 'test_memory.py' -v)
(cd Laravel && python3 -m unittest discover -s memory-bank/tests -p 'test_memory.py' -v)
(cd 'PHP Core' && python3 -m unittest discover -s memory-bank/tests -p 'test_memory.py' -v)
```

Expected: FAIL because policy and documentation do not mention Task Capsules.

- [ ] **Step 3: Add the policy-level phase gate**

Add this common block to the Agent Behavior section of every `AGENTS.md`,
immediately after the existing bounded-context rule:

```markdown
- MUST build a Task Capsule at the start of a complex request and before a
  complex phase handoff. The serialized capsule is limited to 8,000 Unicode
  characters, at most two Procedural, three Semantic, and one Episodic result,
  plus bounded Working state.
- MUST derive a concise sanitized retrieval query from the current request.
  MUST NOT copy the raw request or another prompt into the Task Capsule.
- MUST use a fresh context only at an existing complex boundary: research to
  planning, planning to implementation, implementation to independent
  verification, or recovery after runtime compaction. A simple task stays in
  the current context.
- MUST pass the Task Capsule and explicit current-step files to the fresh
  phase agent. MUST NOT pass the parent conversation, raw agent output, raw
  diffs, logs, prompts, responses, or reasoning.
- MUST progressively open only a cited source required by the current step.
  Repository policy, code, configuration, tests, and specifications remain
  authoritative. Task Capsule creation MUST NOT invoke explicit `complete`.
```

- [ ] **Step 4: Replace the generic Context Handoff list in each Skill Flow**

Keep tool-specific command names elsewhere unchanged. Under the existing
handoff heading, use:

```markdown
## Task Capsule Handoff

At a complex phase boundary, the orchestrating agent builds one bounded Task
Capsule from a concise sanitized retrieval query, optional Working state, and
layered context. A fresh phase agent receives the capsule and explicit
current-step files, not the parent conversation.

The returning handoff contains only:

- work completed;
- decisions made;
- files changed or examined;
- verification evidence;
- the next step;
- unresolved blockers or questions;
- cited authoritative sources.

The next agent must not preload every cited source. It opens one only when the
current step requires more information. Simple tasks remain in the current
context.
```

- [ ] **Step 5: Document the user-visible behavior**

In each edition README and `memory-bank/README.md`, add a concise Task Capsule
section stating:

```markdown
### Task Capsule

At the start of a complex request and before a complex phase handoff, the agent
derives a concise sanitized retrieval query and builds a Task Capsule from
optional Working Memory and `context` retrieval. The raw request is not copied
into the packet. The complete packet is capped at 8,000 Unicode characters and
contains at most two Procedural, three Semantic, and one Episodic result.
Retrieved entries are short snippets with source paths; the next agent reads a
full source only when its current step requires it.

Simple tasks stay in the current context. Fresh contexts are reserved for
research-to-planning, planning-to-implementation,
implementation-to-independent-verification, and recovery after compaction.
`memory`, `checkpoint`, and explicit `complete` keep their existing roles.
```

Use a natural Russian translation in `README_RU.md`, preserving the exact
limits and lifecycle. Add the English block to `README_EN.md`.

- [ ] **Step 6: Add changelog entries**

Under `Unreleased` in all three edition changelogs, record:

```markdown
- Added bounded Task Capsule retrieval and hybrid fresh-context handoffs for
  complex phase boundaries without adding another memory store or changing
  `memory`, `checkpoint`, or explicit `complete`.
```

- [ ] **Step 7: Run the policy tests and verify they pass**

Run the three commands from Step 2.

Expected: PASS.

- [ ] **Step 8: Verify mirror parity and Markdown hygiene**

```bash
cmp Symfony/memory-bank/tests/test_memory.py \
  Laravel/memory-bank/tests/test_memory.py
cmp Symfony/memory-bank/tests/test_memory.py \
  'PHP Core/memory-bank/tests/test_memory.py'
git diff --check
```

Expected: `cmp` exits `0`; `git diff --check` produces no output.

- [ ] **Step 9: Commit policy and documentation**

Stage only the files listed in this task, review `git diff --cached --stat`,
then commit:

```bash
git commit -m "docs: define task capsule phase handoffs"
```

---

### Task 5: Prove the reduction on the real Bauherrenmappe project

**Files:**
- Create: `docs/superpowers/reports/2026-07-29-task-capsule-bauherrenmappe.md`
- Modify: `README_EN.md`
- Modify: `README_RU.md`

**External read-only source:**
- `/home/aliaksei/Desktop/bauherrenmappe`

**Interfaces:**
- Consumes: the mirrored Task Capsule implementation and the three real
  Bauherrenmappe scenarios below.
- Produces: measured baseline/capsule sizes, retained source paths, test
  evidence, and proof that the original checkout stayed unchanged.

- [ ] **Step 1: Capture the original Bauherrenmappe state**

Use a dedicated temporary directory and files rather than broad cleanup paths:

```bash
BENCH_ROOT=$(mktemp -d)
git -C /home/aliaksei/Desktop/bauherrenmappe \
  status --porcelain=v1 -z --untracked-files=all \
  > "$BENCH_ROOT/original-status"
git -C /home/aliaksei/Desktop/bauherrenmappe rev-parse HEAD \
  > "$BENCH_ROOT/original-head"
git -C /home/aliaksei/Desktop/bauherrenmappe write-tree \
  > "$BENCH_ROOT/original-index"
```

If the original `memory-bank/local/context.db` exists, record its SHA-256 in
`"$BENCH_ROOT/original-db"`; otherwise record the literal text `absent`:

```bash
ORIGINAL_DB=/home/aliaksei/Desktop/bauherrenmappe/memory-bank/local/context.db
if test -f "$ORIGINAL_DB"; then
  sha256sum "$ORIGINAL_DB" > "$BENCH_ROOT/original-db"
else
  printf 'absent\n' > "$BENCH_ROOT/original-db"
fi
```

- [ ] **Step 2: Create a disposable real-project clone**

```bash
git clone --local --no-hardlinks \
  /home/aliaksei/Desktop/bauherrenmappe \
  "$BENCH_ROOT/bauherrenmappe"
git -C "$BENCH_ROOT/bauherrenmappe" switch -c task-capsule-benchmark
mkdir -p "$BENCH_ROOT/bauherrenmappe/memory-bank/scripts"
cp Symfony/memory-bank/scripts/context.py \
  "$BENCH_ROOT/bauherrenmappe/memory-bank/scripts/context.py"
cp Symfony/memory-bank/scripts/validate.py \
  "$BENCH_ROOT/bauherrenmappe/memory-bank/scripts/validate.py"
cp -a /home/aliaksei/Desktop/bauherrenmappe/docs \
  "$BENCH_ROOT/bauherrenmappe/docs"
```

The last copy brings the original checkout's untracked password-session design
documents into the disposable clone without modifying the source checkout.

- [ ] **Step 3: Create the three Working scenarios**

Use one external database inside `BENCH_ROOT`:

```bash
CONTEXT_SCRIPT="$BENCH_ROOT/bauherrenmappe/memory-bank/scripts/context.py"
CONTEXT_DB="$BENCH_ROOT/context.db"

python3 "$CONTEXT_SCRIPT" \
  --root "$BENCH_ROOT/bauherrenmappe" --db "$CONTEXT_DB" \
  start --task-id CAPSULE-PASSWORD \
  --goal "Verify password session invalidation and stale remember-me cookies" \
  --file src/GraphQL/Resolver/ChangePasswordResolver.php \
  --file tests/Integration/GraphQL/ChangePasswordTest.php

python3 "$CONTEXT_SCRIPT" \
  --root "$BENCH_ROOT/bauherrenmappe" --db "$CONTEXT_DB" \
  start --task-id CAPSULE-BRANDING \
  --goal "Review Mandant senderEmail branding behavior" \
  --file src/Mailer/AdminMailer.php \
  --file src/Entity/Mandant.php \
  --file tests/Smoke/BrandingTest.php

python3 "$CONTEXT_SCRIPT" \
  --root "$BENCH_ROOT/bauherrenmappe" --db "$CONTEXT_DB" \
  start --task-id CAPSULE-DOCUMENT \
  --goal "Verify ReplaceDocument confidentiality boundaries" \
  --file src/GraphQL/Operation/Mutation/ReplaceDocument.php \
  --file src/Security/DocumentConfidentiality.php \
  --file tests/Integration/GraphQL/DocumentConfidentialityTest.php
```

- [ ] **Step 4: Generate capsules and deterministic baseline measurements**

Run each scenario with `--json`, saving output under `BENCH_ROOT`:

```bash
python3 "$CONTEXT_SCRIPT" \
  --root "$BENCH_ROOT/bauherrenmappe" --db "$CONTEXT_DB" \
  context "password session invalidation remember-me" \
  --task-id CAPSULE-PASSWORD --json \
  > "$BENCH_ROOT/password.json"

python3 "$CONTEXT_SCRIPT" \
  --root "$BENCH_ROOT/bauherrenmappe" --db "$CONTEXT_DB" \
  context "Mandant senderEmail branding" \
  --task-id CAPSULE-BRANDING --json \
  > "$BENCH_ROOT/branding.json"

python3 "$CONTEXT_SCRIPT" \
  --root "$BENCH_ROOT/bauherrenmappe" --db "$CONTEXT_DB" \
  context "ReplaceDocument document confidentiality" \
  --task-id CAPSULE-DOCUMENT --json \
  > "$BENCH_ROOT/document.json"
```

Use this read-only measurement program:

```bash
python3 - "$BENCH_ROOT" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

bench = Path(sys.argv[1])
repository = bench / "bauherrenmappe"
expected = {
    "password": {
        "docs/superpowers/specs/2026-07-23-password-change-session-invalidation-design.md",
    },
    "branding": {"README.md"},
    "document": {"README.md"},
}
results = {}

for scenario in ("password", "branding", "document"):
    capsule_path = bench / f"{scenario}.json"
    capsule_text = capsule_path.read_text(encoding="utf-8").rstrip("\n")
    capsule = json.loads(capsule_text)
    selected_paths = {
        item["path"]
        for layer in ("procedural", "semantic", "episodic")
        for item in capsule[layer]
        if "path" in item
    }
    missing = expected[scenario] - selected_paths
    if missing:
        raise SystemExit(f"{scenario}: missing expected sources: {sorted(missing)}")

    baseline_parts = [
        json.dumps(capsule["working"], ensure_ascii=False, separators=(",", ":"))
    ]
    for path in sorted(selected_paths):
        baseline_parts.append((repository / path).read_text(encoding="utf-8"))
    baseline_chars = len("\n".join(baseline_parts))
    capsule_chars = len(capsule_text)
    reduction = 1 - capsule_chars / baseline_chars
    if capsule_chars > 8000:
        raise SystemExit(f"{scenario}: capsule exceeds 8000 characters")
    if reduction < 0.60:
        raise SystemExit(f"{scenario}: reduction is only {reduction:.1%}")
    results[scenario] = {
        "baseline_chars": baseline_chars,
        "capsule_chars": capsule_chars,
        "reduction_percent": round(reduction * 100, 1),
        "sources": sorted(selected_paths),
    }

(bench / "measurements.json").write_text(
    json.dumps(results, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
print(json.dumps(results, ensure_ascii=False, indent=2))
PY
```

Expected: all three scenarios report at least `60.0` percent reduction, retain
their expected sources, and remain below 8,000 characters.

- [ ] **Step 5: Run the real project verification tests**

Use the original DDEV-served project without editing it:

```bash
cd /home/aliaksei/Desktop/bauherrenmappe
ddev exec php bin/phpunit tests/Integration/GraphQL/ChangePasswordTest.php
ddev exec php bin/phpunit tests/Smoke/BrandingTest.php
ddev exec php bin/phpunit tests/Integration/GraphQL/DocumentConfidentialityTest.php
```

Expected: all three PHPUnit commands PASS. Record each exact command and result
in the report.

- [ ] **Step 6: Prove the original checkout is unchanged**

```bash
git -C /home/aliaksei/Desktop/bauherrenmappe \
  status --porcelain=v1 -z --untracked-files=all \
  > "$BENCH_ROOT/after-status"
git -C /home/aliaksei/Desktop/bauherrenmappe rev-parse HEAD \
  > "$BENCH_ROOT/after-head"
git -C /home/aliaksei/Desktop/bauherrenmappe write-tree \
  > "$BENCH_ROOT/after-index"
cmp "$BENCH_ROOT/original-status" "$BENCH_ROOT/after-status"
cmp "$BENCH_ROOT/original-head" "$BENCH_ROOT/after-head"
cmp "$BENCH_ROOT/original-index" "$BENCH_ROOT/after-index"
```

Recompute the original database hash or `absent` state and compare it with
`"$BENCH_ROOT/original-db"`:

```bash
if test -f "$ORIGINAL_DB"; then
  sha256sum "$ORIGINAL_DB" > "$BENCH_ROOT/after-db"
else
  printf 'absent\n' > "$BENCH_ROOT/after-db"
fi
cmp "$BENCH_ROOT/original-db" "$BENCH_ROOT/after-db"
```

Expected: every comparison exits `0`.

- [ ] **Step 7: Write the evidence report**

Create
`docs/superpowers/reports/2026-07-29-task-capsule-bauherrenmappe.md` with:

- the source checkout path and captured branch/HEAD;
- one table row per scenario containing `baseline_chars`, `capsule_chars`,
  `reduction_percent`, and selected source paths from
  `"$BENCH_ROOT/measurements.json"`;
- the three exact PHPUnit commands and their pass/fail results;
- confirmation that status, HEAD, index tree, and local database state matched;
- a statement that runtime token counts were unavailable unless a runtime
  actually reported them;
- no source file contents, secrets, or `.env` data.

Update `README_EN.md` and `README_RU.md` with a one-paragraph summary and a
relative link to the report.

- [ ] **Step 8: Remove only the verified disposable directory**

Validate that `BENCH_ROOT` is non-empty and points beneath the system temporary
directory, then remove that exact directory. Do not use a broad glob or a
workspace path.

```bash
case "$BENCH_ROOT" in
  /tmp/*)
    test -d "$BENCH_ROOT"
    rm -rf -- "$BENCH_ROOT"
    ;;
  *)
    printf 'Refusing unsafe benchmark cleanup path\n' >&2
    exit 1
    ;;
esac
```

- [ ] **Step 9: Run final repository verification**

```bash
(cd Symfony && python3 -m unittest discover -s memory-bank/tests -q)
(cd Laravel && python3 -m unittest discover -s memory-bank/tests -q)
(cd 'PHP Core' && python3 -m unittest discover -s memory-bank/tests -q)
(cd Symfony && python3 memory-bank/scripts/validate.py)
(cd Laravel && python3 memory-bank/scripts/validate.py)
(cd 'PHP Core' && python3 memory-bank/scripts/validate.py)
cmp Symfony/memory-bank/scripts/context.py \
  Laravel/memory-bank/scripts/context.py
cmp Symfony/memory-bank/scripts/context.py \
  'PHP Core/memory-bank/scripts/context.py'
cmp Symfony/memory-bank/tests/test_context.py \
  Laravel/memory-bank/tests/test_context.py
cmp Symfony/memory-bank/tests/test_context.py \
  'PHP Core/memory-bank/tests/test_context.py'
cmp Symfony/memory-bank/tests/test_memory.py \
  Laravel/memory-bank/tests/test_memory.py
cmp Symfony/memory-bank/tests/test_memory.py \
  'PHP Core/memory-bank/tests/test_memory.py'
git diff --check
```

Expected: all suites and validators PASS, mirrors are byte-identical, and
`git diff --check` is silent.

- [ ] **Step 10: Commit the real-project evidence**

```bash
git add docs/superpowers/reports/2026-07-29-task-capsule-bauherrenmappe.md \
  README_EN.md README_RU.md
git commit -m "docs: record task capsule pressure test"
```

---

## Final Verification Checklist

- [ ] `context` without `--task-id` returns a request-only capsule.
- [ ] Unknown Working task IDs warn instead of aborting capsule retrieval.
- [ ] Working goal/progress/files enrich retrieval without entering shell syntax.
- [ ] Capsule output is deterministic and no larger than 8,000 characters.
- [ ] Layer maxima are 2 Procedural, 3 Semantic, and 1 Episodic.
- [ ] Duplicate paths and episode IDs are removed.
- [ ] Optional results are dropped before goal or Procedural constraints.
- [ ] Index failure returns request/Working-only context and does not surface stale results.
- [ ] No parent transcript, raw diff, logs, prompts, responses, reasoning, secrets, ignored contents, or full source documents enter the capsule.
- [ ] `memory`, `checkpoint`, manual lifecycle commands, and atomic `complete` remain compatible.
- [ ] Hybrid phase boundaries are documented without creating a new command or skill.
- [ ] All three implementation and test mirrors are byte-identical.
- [ ] All edition test suites and validators pass.
- [ ] Three Bauherrenmappe scenarios each reduce transferred text by at least 60% and retain required sources.
- [ ] Bauherrenmappe PHPUnit verification passes and the original checkout state is unchanged.
