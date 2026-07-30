# Context Brain + Local Context Engine — Reconciled Architecture

This document explains the system that results from merging **PR #6
(`local-context-engine`)** and **PR #8 (`feature/context-brain`)** into `main`: what the
merged system is, why the two PRs collided, how the collision was resolved, and what
still needs doing.

Every measurement below was produced by executing the merged tree, not by reading diffs.

---

## 1. Why the two PRs overlapped

PR #8 was branched from **PR #6's branch**, not from `main`, at commit `2723110` —
*before* PR #6's nine-commit "task capsule" series landed. It then merged `main` in twice
without ever picking up PR #6's later commits.

```
main (7eab3ae) ──────────────────────────────────────────────►
     ▲                                              │ both PRs target main
2723110  "docs: add English and Russian readmes"   ← the real fork point
     ├──► 9 commits (task capsule)  ──────────────► PR #6   104 files, +19527/-297
     └──► 3 commits (context brain) ──────────────► PR #8   319 files, +31445/-471
```

Consequences worth internalising:

- The ~44 "shared" commits are not duplicated work. They are simply everything up to
  `2723110`, present in both branches because one is an ancestor-fork of the other.
- **PR #8 contains no task-capsule code at all.** Where PR #8 appears to *delete* PR #6's
  work, that is divergence, not intent.
- Both PRs restructured the same `main()` dispatch region of
  `memory-bank/scripts/context.py` from an ancestor that predates one of them.
- The result is **29 conflicted files, identical in either merge order**. Whichever PR
  lands first merges clean; the second always hits the same 29.

---

## 2. What the merged system is

One context engine per accelerator (`Laravel/`, `Symfony/`, `PHP Core/`), exposing
**two modes over four memory layers** behind a single CLI facade.

```
<accelerator>/
├── memory-bank/
│   ├── scripts/
│   │   ├── context.py            1743 lines  CLI facade + lightweight engine   (PR #6 + merged dispatch)
│   │   ├── brain_runtime.py      1305 lines  governed record/task runtime      (PR #8)
│   │   ├── context_retrieval.py   569 lines  governed budgeted retrieval       (PR #8)
│   │   └── validate.py            349 lines  dependency-free validator         (shared)
│   ├── chunks/                    durable, human-approved project knowledge
│   └── local/context.db           gitignored SQLite FTS5 index + local task state
└── project-brain/                 governed authority: tasks, records, handoffs, manifests
```

### The two modes

| | **governed** (default) | **lightweight** (`--mode lightweight`) |
|---|---|---|
| Authority | `project-brain/` — revision-safe, on disk, reviewable | local SQLite only |
| Tasks | six governed record types, CAS updates, handoffs | `working_tasks` table |
| Retrieval | `retrieve()` + retrieval manifests | `build_context_packet()` + Task Capsule |
| Budget | **8,000 / 12,000 tokens**, 1,200-char snippets | **8,000 characters** |
| Use for | non-trivial work needing shared state and continuity | local-only work, no formal handoff |

Mode is resolved by `configured_mode()` from, in order: `--mode`, `$PROJECT_BRAIN_MODE`,
then `project-brain/config/runtime.json` (`"mode": "governed"` as shipped).

### Four memory layers

`procedural` (policy, skills) · `semantic` (specs, docs, active memory chunks) ·
`episodic` (changelog, completed work) · `working` (current task state).

The lightweight Task Capsule bounds them to 2 / 3 / 1 results respectively, inside the
8,000-character cap.

---

## 3. The central design decision of the merge

**The two budget systems stay separate. `enforce_capsule_budget` runs on the lightweight
branch only.**

This is the one thing that is easy to get wrong and that the test suite will not catch.

PR #6 caps the capsule at 8,000 **characters**. PR #8's `retrieve()` independently caps at
8,000 **tokens** (hard limit 12,000). Those are not the same limit — 8,000 tokens is
roughly 32,000 characters. `retrieve()` also returns `categories` and `selected`, which
dominate the payload and which the character-budget loop never pops.

Measured: a governed payload of 6 items × 800-character snippets — 18,529 characters raw,
comfortably inside PR #8's own token budget — raises
`ContextError: mandatory Task Capsule content exceeds 8000 characters` if the character
cap is applied on top.

So the merged dispatch is:

```python
if arguments.command in {"context", "retrieve"}:
    if mode == "lightweight":
        ...
        result = build_context_packet(..., include_retrieval=..., warnings=warnings)
        result = enforce_capsule_budget(result)      # character cap — ONLY here
    else:
        if arguments.task_id is None:
            raise ContextError("Governed retrieval requires --task-id")
        binding = governed_binding(connection, arguments.task_id)
        result = retrieve(...)                        # token budget applies internally
        result["episodic"] = (...)[: arguments.limit]
        result.setdefault("warnings", [])             # retrieve() has no such key
```

Three further constraints, each verified by execution:

1. `result.setdefault("warnings", [])` — `retrieve()` returns 12 keys, none named
   `warnings`; the auto-merged print tail dereferences `result["warnings"]`, so without
   this every non-`--json` governed call raises `KeyError`. No test covers non-`--json`.
2. The `task_id is None` guard — PR #6 made `--task-id` optional on `context` while
   `retrieve` requires it. `validate_task_id(None)` raises `AttributeError`, which is not
   in `main()`'s except tuple, so it surfaces as a raw traceback.
3. No index refresh on the governed branch —
   `test_stale_source_is_filtered_before_reindex_and_retrieval` depends on the existing
   explicit-index ordering.

Neither side of the conflict works alone:

| Resolution | Result |
|---|---|
| `--ours` (PR #6) | `retrieve` → `context: Unsupported command: retrieve` |
| `--theirs` (PR #8) | capsule budget gone — `AssertionError: 13149 not less than or equal to 8000` |
| union | `test_memory.py` → `IndentationError` |

---

## 4. Resolving the merge

29 conflicted files collapse to **13 distinct edits**, because `context.py` and
`test_memory.py` are byte-identical across all three accelerators on every branch —
resolve once, `cp` twice. The prose files genuinely differ per accelerator.

| File | Identical ×3? | Edits |
|---|---|---|
| `memory-bank/scripts/context.py` | whole file | 1 → `cp` ×2 |
| `memory-bank/tests/test_memory.py` | whole file | 1 → `cp` ×2 |
| `memory-bank/README.md` | hunk only | 3 |
| `{.agents,.claude,.cursor}/skills/SKILL FLOW.md` | within accelerator | 3 → 9 files |
| `AGENTS.md`, `CHANGELOG.md`, `README.md` | no | 3 each |
| `README_EN.md`, `README_RU.md` (root) | n/a | 2 |

### Two fixes git will never flag

These produce **no conflict markers**. They are the dangerous part of this merge.

**(a) `git_ignored_paths` fail-closed breaks 18 of PR #8's tests.**
PR #6 commit `a0a9177` hardened the probe from fail-**open** (`return set()`) to
fail-**closed** (`raise ContextError`). PR #8 never touched the function, so git
auto-merges PR #6's version cleanly — and PR #8's harness creates a plain
`tempfile.TemporaryDirectory` that is not a git repo, so `check-ignore` exits 128.

| Tree | `project-brain/tests/test_runtime.py` |
|---|---|
| PR #8 alone | 22/22 **OK** ×3 |
| after merge | **5 failures + 1 error** ×3 = **18** |

Keep fail-closed — a fail-open probe means gitignored secrets get indexed, and PR #6 ships
tests asserting the raise. Fix the harness instead, in all three
`project-brain/tests/test_runtime.py`, right after `self.repository = Path(self.temporary.name)`:

```python
subprocess.run(["git", "init", "--quiet", str(self.repository)], check=True, capture_output=True)
```

`subprocess` is already imported; the anchor occurs exactly once. Verified: restores 22/22 ×3.

**(b) `reject_capsule_privacy` is bypassed on the governed path.**
The gap exists in neither branch alone — the merge creates it.
`start_working_task`/`update_working_task` call the guard, but PR #8's dispatch only routes
there in lightweight mode. Governed is the shipped default:

```
$ context.py start --task-id PII-1 --goal "user: email john.doe@example.com about customer name: Jane Roe"
→ exit 0, persisted verbatim

$ context.py --mode lightweight start …   (same string)
→ context: Working task contains private or raw data; replace it with a sanitized summary
```

Fix by hoisting `reject_capsule_privacy(...)` into the shared pre-dispatch validation in
`main()`, next to `reject_secrets`. That covers `start`/`update` too — which merge
*cleanly*, so a reviewer working through conflict markers will never see them.

### A blind spot to know about

`memory-bank/tests/test_context.py` **auto-merges with no conflict**, yet both sides
touched it (PR #6 +671 lines, PR #8 +15/−3). Worse, its `run_context()` harness hardcodes
`--mode lightweight`, so every capsule test exercises the *non-default* path. The governed
path has zero capsule coverage. A green suite does not mean the governed path works —
this is exactly how a wrong budget resolution survives review.

Before landing, diff it against both parents:

```bash
git diff 2723110 origin/local-context-engine  -- '*/memory-bank/tests/test_context.py'
git diff 2723110 origin/feature/context-brain -- '*/memory-bank/tests/test_context.py'
```

---

## 5. Using the merged engine

```bash
# governed (default) — project-brain is authoritative
python3 memory-bank/scripts/context.py start --task-id PROJ-133 --goal "Invalidate other sessions"
python3 memory-bank/scripts/context.py retrieve "password session invalidation" --task-id PROJ-133
python3 memory-bank/scripts/context.py update --task-id PROJ-133 --progress "Regression passes"
python3 memory-bank/scripts/context.py complete --task-id PROJ-133 \
  --outcome "Sessions invalidated" --verification "ChangePasswordTest passed"

# lightweight — local SQLite only, bounded Task Capsule
python3 memory-bank/scripts/context.py --mode lightweight context "session invalidation" --task-id PROJ-133
```

`context` and `retrieve` share a dispatch; `context` additionally refreshes the index in
lightweight mode. Governed records use `brain-create` / `brain-update` / `brain-get`, and
durable promotion runs `promote-propose` → `promote-review` → `promote-apply`.

Full surface: `index, search, context, retrieve, record, start, update, get, clear,
complete, brain-create, brain-update, brain-get, status, validate, parity, compact,
promote-propose, promote-review, promote-apply`.

---

## 6. Verification

Per accelerator, from the accelerator root:

```bash
cd memory-bank/tests && for t in test_*.py; do python3 "$t"; done          # 79 OK
cd ../.. && python3 -m unittest discover -s project-brain/tests -p "test_runtime.py"   # 22 OK
```

Merged tree with all fixes above: **303/303 green** (79 + 22, × 3 accelerators).

| Check | Command | Expected |
|---|---|---|
| No leftover markers | `git grep -n '^<<<<<<<\|^>>>>>>>' -- '*.md' '*.py'` | empty |
| `context.py` identical ×3 | `md5sum */memory-bank/scripts/context.py` | 1 distinct hash |
| Budget call site survives | `grep -c 'enforce_capsule_budget(result)' <acc>/memory-bank/scripts/context.py` | `1` |
| …and only on lightweight | it must sit inside `if mode == "lightweight"` | yes |
| Governed non-JSON works | `context.py context "memory" --task-id V-1` | prints, no `KeyError` |
| Governed privacy gate | `context.py start --task-id V-2 --goal "user: mail a@b.com"` | non-zero exit |
| Fail-closed preserved | `grep -c 'Git ignore probe failed with exit status' <acc>/memory-bank/scripts/context.py` | `1` |

Deliberately **not** a merge gate: `context.py parity` exits 1 on all three accelerators —
it is red on `main` and PR #6 too, and PR #8 actually improves it.

---

## 7. Known gaps and follow-ups

### Prose still to merge

The code merge and the test-critical doc merge are settled. Two files still carry only
PR #6's side and need PR #8's governed prose folded back in:

- `CHANGELOG.md` (×3) — take PR #8's `### Added` block, then place PR #6's capsule bullet
  *inside* it.
- `README_EN.md` / `README_RU.md` — take PR #8's body, re-insert PR #6's `### Task Capsule`
  immediately **before** `### Explicit Governed CLI` (after it separates the heading from
  its bash block).

Also: PR #8 deleted the `## Bauherrenmappe Verification` heading while PR #6's pressure-test
paragraph merged cleanly beneath it, leaving text with no antecedent in both root READMEs.
No test catches this — the orphan contains "Task Capsule" and helps the doc test pass.

`AGENTS.md` is resolved as an explicit union: PR #8's governed-mode bullets, then PR #6's
Task Capsule bullets. Note the coupling — `test_memory.py` pins exact strings from **both**
sides, so resolve the docs before the tests.

### Product bugs confirmed in the PRs (not merge artifacts)

| Issue | Owner |
|---|---|
| `update_record()` unconditionally recomputes `source_fingerprints()`; a renamed/deleted cited source permanently bricks the record (`sources` is append-only), and `compact()` then refuses repo-wide. `docs/TROUBLESHOOTING.md:317-345` prescribes the exact path the code blocks. | PR #8 |
| `bind_governed_task()` inserts positional `(external_id, task_uuid, ts, ts)` into columns `(task_id, goal, …)` — the **UUID lands in `goal`**. Lightweight `get` returns it as the goal; `complete` writes it as an episode summary, creating a second divergent authority for a live governed task. | PR #8 |
| Episodic memory is unreachable via the capsule: `episodic_limit = min(limit, 1)`, filled documents-first, and the shipped 19.5 KB `CHANGELOG.md` matches nearly every query via OR-joined FTS. `test_context.py:1113-1174` *asserts* the collision, so it must be amended. | PR #6 |

### Repo-level

1. **No CI on any branch** — 303 tests exist and nothing runs them. Add a matrix over
   `[Laravel, Symfony, 'PHP Core']` (**quote the space**); stdlib-only, no dependencies.
2. `memory-bank/tests/` has no `__init__.py`, so `unittest discover` fails there while
   `project-brain/tests` works.
3. Accelerator suites read `REPOSITORY_ROOT.parent/README*.md`, so the shipped suite fails
   under the documented "copy into your project" install.
4. ~25–40 occurrences of a developer's absolute home path, a client ticket ID, a commit SHA
   and a source-file table are committed under `docs/superpowers/` and linked from both root
   READMEs. **Confirm repo visibility** — if public, this is disclosure remediation, not cleanup.
5. Consumers will commit `memory-bank/local/context.db`; the install list never mentions
   `.gitignore`. Add a nested `<accel>/memory-bank/.gitignore` containing `local/`.
6. `import fcntl` at `brain_runtime.py:7` module scope → the CLI is unimportable on native Windows.
7. `PHP Core/` contains a space — breaks unquoted CI steps and `xargs` without `-0`.

### Reviewer comments on PR #6

**"Why is framework-agnostic code under `Symfony/`?"** — Correct, and understated: 19 of the
29 files PR #6 touches under `Symfony/` are byte-identical to their twins, 18 with zero
Symfony vocabulary. But correct the framing: this causes **6 of 29 conflicts (21%)**, not
most of them — 21 conflicts are accelerator-specific prose that cannot be collapsed.
Extraction takes 29 → 25. Do it *after* this merge.

**"Too much Russian"** — Partially. EN/RU are currently in perfect sync (19 headings each).
The real regression is that root `README.md` — the GitHub landing page — was reduced from
106 lines to a 3-line language switcher, and GitHub does not fall back to `README_EN.md`.
Restore the English content into `README.md`, keep `README_RU.md` linked from a banner
marked "README.md is authoritative".

---

## 8. Preventing a recurrence

PR #8 branched from PR #6's branch and then merged `main` in without ever picking up
PR #6's later commits, so both PRs rewrote the same dispatch from an ancestor that
predated one of them — and the divergence read as deletion.

1. **Branch from `main`.** When a dependency on an unmerged branch is genuinely needed,
   declare it in the PR description and **rebase — never merge — onto the dependency's tip**
   before requesting review, so the dependency's later commits are always present rather
   than divergent.
2. **Treat `memory-bank/scripts/context.py` as a serialization point.** It is byte-identical
   across three accelerators, has no CI, and is the one file both PRs restructured. Landing
   CI plus the shared-engine extraction means the next collision conflicts once instead of
   three times, and a red suite blocks the merge instead of waiting for someone to type
   `python3 -m unittest` by hand.
