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
- A native PHP convention (PSR, boundary validation, parameterized queries) is violated repeatedly.
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

## Native PHP Examples

### Rule: Validate At The Boundary

**Trigger:** A handler used raw request data directly.
**Root cause:** Skill guidance did not require validating/normalizing input into a typed DTO.
**Rule:** MUST validate and normalize input into a typed request DTO/value object before using it.
**Example:**
- Incorrect: `$this->createUser->handle($request->getParsedBody())`
- Correct: `$this->createUser->handle(CreateUserRequest::fromArray((array) $request->getParsedBody()))`
**Enforcement:** `AGENTS.md`, `coder` skill, code review checklist.

### Rule: Authorization Is Server-Side

**Trigger:** A UI button was hidden, but the endpoint was still callable.
**Root cause:** Authorization was treated as a frontend concern.
**Rule:** MUST enforce protected actions in an explicit server-side access-control layer.
**Example:**
- Incorrect: hide the Delete button only.
- Correct: check `$accessControl->assertCan($user, 'delete', $resource)` in the handler/use case.
**Enforcement:** `AGENTS.md`, `architect`, `coder`, `code-reviewer`.

### Rule: Queries Must Be Parameterized

**Trigger:** A query concatenated user input into SQL.
**Root cause:** No rule mandated prepared statements.
**Rule:** MUST use PDO prepared statements with bound parameters; never concatenate untrusted input into SQL.
**Example:**
- Incorrect: `$pdo->query("SELECT * FROM users WHERE email = '$email'")`
- Correct: `$stmt = $pdo->prepare('SELECT * FROM users WHERE email = :email'); $stmt->execute(['email' => $email]);`
**Enforcement:** `AGENTS.md`, `coder`, `code-reviewer`, `security-reviewer`.

## Verification

After adding a rule:

1. Read the target file back.
2. Confirm the rule does not conflict with existing policy.
3. If possible, add or update a hook/checklist item.
4. Run `/verify` if enforcement behavior changed.
