# Local Context Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add dependency-free repository-local context search and episodic task memory to the Laravel, PHP Core, and Symfony accelerators.

**Architecture:** Each accelerator owns an identical Python CLI backed by a gitignored SQLite database. Git-tracked documents remain authoritative; the database is a disposable FTS5 index plus non-authoritative local task episodes.

**Tech Stack:** Python 3.9+ standard library, SQLite FTS5, `unittest`, existing memory-bank validator metadata and secret checks

## Global Constraints

- Keep `Laravel/`, `PHP Core/`, and `Symfony/` self-contained.
- Add no runtime dependency or network service.
- Support Python 3.9 and newer.
- Store the database only at `memory-bank/local/context.db` by default.
- Never store raw conversations or secret-like values.
- Keep production scripts and their tests byte-identical across all three accelerators.
- Do not modify `Infrastructure-Creator` in this first version.

---

### Task 1: Symfony Context CLI

**Files:**
- Create: `Symfony/memory-bank/scripts/context.py`
- Create: `Symfony/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: `validate.SECRET_PATTERNS`
- Produces: CLI commands `index`, `search`, `record`, and `status`

- [ ] **Step 1: Write the failing indexing and search test**

Create a temporary repository containing `AGENTS.md`, a living specification,
and active/superseded memory chunks. Invoke the real CLI:

```python
result = subprocess.run(
    [sys.executable, str(SCRIPT), "--root", str(repository), "index", "--json"],
    text=True,
    capture_output=True,
)
self.assertEqual(0, result.returncode, result.stderr)

search = subprocess.run(
    [sys.executable, str(SCRIPT), "--root", str(repository), "search", "invoice ownership", "--json"],
    text=True,
    capture_output=True,
    check=True,
)
payload = json.loads(search.stdout)
self.assertEqual(["specs/billing.md"], [item["path"] for item in payload["documents"]])
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
python3 -m unittest Symfony/memory-bank/tests/test_context.py -v
```

Expected: failure because `memory-bank/scripts/context.py` does not exist.

- [ ] **Step 3: Implement indexing and search**

Create the SQLite schema, bounded source discovery, canonically validated
active-memory filtering, transactional document-index rebuild, safe FTS5 query
construction, JSON output, and human-readable output in `context.py`.

- [ ] **Step 4: Run the test and verify GREEN**

Run:

```bash
python3 -m unittest Symfony/memory-bank/tests/test_context.py -v
```

Expected: the indexing/search test passes.

- [ ] **Step 5: Add the failing episode test**

Invoke `record` with a summary, outcome, file, verification, and source, then
search for a term unique to the episode and assert that it appears under
`episodes`.

- [ ] **Step 6: Run the episode test and verify RED**

Run the same unittest command. Expected: failure because `record` is not
implemented.

- [ ] **Step 7: Implement episode recording**

Validate required fields, reuse `SECRET_PATTERNS`, store JSON lists in SQLite,
and return the new episode ID without echoing stored content.

- [ ] **Step 8: Run the episode test and verify GREEN**

Run the same unittest command. Expected: all current tests pass.

- [ ] **Step 9: Add failing stale-index and secret-rejection tests**

Assert that re-indexing removes a deleted source. Attempt to record a synthetic
GitHub token and assert a non-zero exit, a redacted error, and zero stored
episodes.

- [ ] **Step 10: Run the new tests and verify RED**

Run the same unittest command. Expected: failures for missing stale-row cleanup
or missing secret rejection.

- [ ] **Step 11: Implement the remaining behavior**

Delete database document rows absent from the current source set and reject
secret-pattern matches before insertion.

- [ ] **Step 12: Run the complete Symfony memory-bank tests**

```bash
python3 -m unittest discover -s Symfony/memory-bank/tests -v
```

Expected: all validator and context tests pass.

### Task 2: Mirror The Proven Implementation

**Files:**
- Create: `Laravel/memory-bank/scripts/context.py`
- Create: `Laravel/memory-bank/tests/test_context.py`
- Create: `PHP Core/memory-bank/scripts/context.py`
- Create: `PHP Core/memory-bank/tests/test_context.py`

**Interfaces:**
- Consumes: the verified Symfony implementation
- Produces: byte-identical context behavior in all three accelerators

- [ ] **Step 1: Mirror the verified script and tests**

Copy the Symfony files without framework-specific changes.

- [ ] **Step 2: Verify mirror parity**

```bash
cmp Symfony/memory-bank/scripts/context.py Laravel/memory-bank/scripts/context.py
cmp Symfony/memory-bank/scripts/context.py "PHP Core/memory-bank/scripts/context.py"
cmp Symfony/memory-bank/tests/test_context.py Laravel/memory-bank/tests/test_context.py
cmp Symfony/memory-bank/tests/test_context.py "PHP Core/memory-bank/tests/test_context.py"
```

Expected: every command exits zero.

- [ ] **Step 3: Run all three memory-bank test suites**

```bash
for edition in Laravel "PHP Core" Symfony; do
  python3 -m unittest discover -s "$edition/memory-bank/tests" -v
done
```

Expected: all suites pass.

### Task 3: Integrate The Context Workflow

**Files:**
- Modify: `Laravel/memory-bank/README.md`
- Modify: `PHP Core/memory-bank/README.md`
- Modify: `Symfony/memory-bank/README.md`
- Modify: `Laravel/{.claude/skills,.cursor/skills,.agents/skills}/memory-bank/SKILL.md`
- Modify: `PHP Core/{.claude/skills,.cursor/skills,.agents/skills}/memory-bank/SKILL.md`
- Modify: `Symfony/{.claude/skills,.cursor/skills,.agents/skills}/memory-bank/SKILL.md`
- Modify: `Laravel/CHANGELOG.md`
- Modify: `PHP Core/CHANGELOG.md`
- Modify: `Symfony/CHANGELOG.md`

**Interfaces:**
- Consumes: the CLI contract from Task 1
- Produces: discoverable cross-tool usage guidance

- [ ] **Step 1: Document local context commands**

Add the four commands, authority boundary, local-only episode policy, and
FTS5-only ceiling to each memory-bank README.

- [ ] **Step 2: Add Context mode to every memory-bank skill mirror**

Add `Context index/search/record/status` as a mode, require source verification
before durable capture, and prohibit raw transcript recording.

- [ ] **Step 3: Record the feature in each edition changelog**

Add one concise entry describing local SQLite/FTS5 context search and episodic
memory.

- [ ] **Step 4: Verify skill mirror parity inside each edition**

Normalize only the expected framework wording, then compare the new Context
workflow section across `.claude`, `.cursor`, and `.agents`.

- [ ] **Step 5: Run final checks**

```bash
for edition in Laravel "PHP Core" Symfony; do
  python3 "$edition/memory-bank/scripts/validate.py"
  python3 "$edition/memory-bank/scripts/context.py" --root "$edition" index --json
  python3 "$edition/memory-bank/scripts/context.py" --root "$edition" search memory --json
done
git diff --check
```

Expected: validators and CLI commands exit zero, and `git diff --check` reports
no whitespace errors.
