# Stabilization - Symfony Layered Architecture

Use this document to turn repeated agent mistakes, workflow friction, and user corrections into durable Symfony rules.

## Cycle

```text
Incident -> Root Cause -> Rule -> Example -> Enforcement -> Verification
```

## When To Stabilize

- The same mistake happens more than once.
- A user correction reveals a missing rule.
- A hook blocks too much or too little.
- A Symfony convention is violated repeatedly.
- Controller -> Service -> Repository boundaries are violated repeatedly.
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

## Symfony Examples

### Rule: Validate At The Boundary

**Trigger:** A controller used raw request data directly.
**Root cause:** Skill guidance did not require Symfony Forms, Validator, request DTOs, or explicit validation before service calls.
**Rule:** MUST validate and normalize Symfony HTTP input before using it.
**Example:**
- Incorrect: `$service->create($request->request->all())`
- Correct: `$service->create($createInvitationRequest)` after `#[MapRequestPayload]`, Form handling, Validator, or explicit DTO validation.
**Enforcement:** `AGENTS.md`, `coder` skill, code review checklist.

### Rule: Authorization Is Server-Side

**Trigger:** A UI button was hidden, but the route was still callable.
**Root cause:** Authorization was treated as a frontend concern.
**Rule:** MUST enforce protected actions with Symfony voters, `access_control`, security attributes, or explicit server-side checks.
**Example:**
- Incorrect: hide the Delete button only.
- Correct: `$this->denyAccessUnlessGranted('DELETE', $resource)` in the controller or a dedicated service authorization check.
**Enforcement:** `AGENTS.md`, `architect`, `coder`, `code-reviewer`, `security-reviewer`.

### Rule: Doctrine Queries Must Be Bound

**Trigger:** A Doctrine query concatenated user input into DQL/SQL.
**Root cause:** No rule mandated parameters for QueryBuilder/DQL/raw SQL.
**Rule:** MUST bind Doctrine parameters; never concatenate untrusted input into DQL or SQL.
**Example:**
- Incorrect: `->andWhere("user.email = '$email'")`
- Correct: `->andWhere('user.email = :email')->setParameter('email', $email)`
**Enforcement:** `AGENTS.md`, `coder`, `repository-reviewer`, `code-reviewer`, `security-reviewer`.

### Rule: Controllers Are Not Use Cases

**Trigger:** A controller performed validation, business decisions, Doctrine writes, and side effects.
**Root cause:** The implementation skipped the service layer.
**Rule:** MUST move multi-step workflows into an application service.
**Example:**
- Incorrect: controller queries Doctrine, creates entity, flushes, sends mail, returns entity.
- Correct: controller validates/authorizes, calls `CreateInvitationService::create()`, returns a response DTO.
**Enforcement:** `AGENTS.md`, `coder`, `architecture-boundary-reviewer`, `code-reviewer`.

## Verification

After adding a rule:

1. Read the target file back.
2. Confirm the rule does not conflict with existing policy.
3. If possible, add or update a hook/checklist item.
4. Run `/verify` if enforcement behavior changed.
