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
- A Laravel/PHP convention is violated repeatedly.
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

### Rule: Validate At HTTP Boundary

**Trigger:** Controller accepted raw request data directly.
**Root cause:** Skill guidance did not require Form Requests for non-trivial input.
**Rule:** MUST validate Laravel HTTP input with a Form Request, validator, or explicit validation before using it.
**Example:**
- Incorrect: `User::create($request->all())`
- Correct: `User::create($request->validated())`
**Enforcement:** `AGENTS.md`, `coder` skill, code review checklist.

### Rule: Policies Are Server-Side Authorization

**Trigger:** A UI button was hidden, but the API action was still callable.
**Root cause:** Authorization was treated as a frontend concern.
**Rule:** MUST enforce protected actions with Laravel policies, gates, or middleware.
**Example:**
- Incorrect: hide the Delete button only.
- Correct: call `$this->authorize('delete', $model)` or use policy-backed Form Request authorization.
**Enforcement:** `AGENTS.md`, `architect`, `coder`, `code-reviewer`.

## Verification

After adding a rule:

1. Read the target file back.
2. Confirm the rule does not conflict with existing policy.
3. If possible, add or update a hook/checklist item.
4. Run `/verify` if enforcement behavior changed.
