# Stabilization

Use this document to turn repeated agent mistakes, workflow friction, and user corrections into durable rules.

## Cycle

```text
Incident -> Root Cause -> Rule -> Example -> Enforcement -> Verification
```

## When To Stabilize

- The same mistake happens more than once.
- A user correction reveals a missing rule.
- A hook blocks too much or too little.
- A Laravel convention (Form Request validation, Policy authorization, Eloquent parameter binding) is violated repeatedly.
- A workflow handoff is confusing.

## Rule Template

```markdown
### Rule: [Short Name]

**Trigger:** [What happened]
**Root cause:** [Why it happened]
**Rule:** MUST/MUST NOT [enforceable behavior]
**Example:**
- Incorrect: [bad example]
- Correct: [good example]
**Enforcement:** Policy / skill instruction / hook / review checklist
**Added:** YYYY-MM-DD
```

## Laravel Examples

### Rule: Validate Via Form Requests

**Trigger:** A controller read `$request->input(...)` directly and acted on it without validation.
**Root cause:** Skill guidance did not require a Form Request at the boundary.
**Rule:** MUST validate input via a Form Request (or explicit validator) before using it in a controller.
**Example:**
- Incorrect: `$user = User::create($request->all());`
- Correct: `$user = User::create($request->validated());` with a `StoreUserRequest` defining the rules.
**Enforcement:** `AGENTS.md`, `coder` skill, code review checklist.

### Rule: Authorization Is Server-Side

**Trigger:** A UI button was hidden, but the endpoint was still callable.
**Root cause:** Authorization was treated as a frontend concern.
**Rule:** MUST enforce protected actions via a Policy or Gate checked in the controller/Action, not by hiding UI.
**Example:**
- Incorrect: hide the Delete button only.
- Correct: `$this->authorize('delete', $post);` (or `Gate::authorize(...)`) before acting.
**Enforcement:** `AGENTS.md`, `architect`, `coder`, `code-reviewer`.

### Rule: Avoid N+1 Queries

**Trigger:** A Blade view or API Resource looped over a relationship inside a collection, issuing one query per row.
**Root cause:** No eager loading was applied before the loop.
**Rule:** MUST eager-load relationships (`with()`/`load()`) that will be accessed across a collection.
**Example:**
- Incorrect: `Post::all()` then `$post->author->name` inside the view loop.
- Correct: `Post::with('author')->get()` before the loop.
**Enforcement:** `AGENTS.md`, `coder`, `performance-optimization`, `code-reviewer`.

## Verification

After adding a rule:

1. Read the target file back.
2. Confirm the rule does not conflict with existing policy.
3. If possible, add or update a hook/checklist item.
4. Run `/verify` if enforcement behavior changed.
