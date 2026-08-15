# Stabilization

Use this document to turn repeated agent mistakes, workflow friction, and user corrections into durable rules.

## Cycle

```text
Incident -> Localization -> Root Cause -> Rule -> Example -> Enforcement -> Verification
```

## Localization

Before naming a root cause, localize the failure: which two components interacted, and which of them is responsible. The same visible symptom needs a different repair depending on the answer, so an incident that is never localized routes to the same place every time - one more sentence in a policy document - even when nothing the agent could read would have prevented it.

Record the localization as `COMPONENT - COMPONENT | blame: SIDE`, using these components:

| Component | In this accelerator |
|-----------|---------------------|
| Model | the agent's own policy: its plan, reasoning, tool choice, and adherence to what it was told |
| Owner | the user and the request they actually made |
| Grader | `DOD.md`, tests, static analysis, CI - whatever decides the work is done |
| Third party | content the run ingested that nobody in this repository wrote: PR and issue text, fetched pages, package metadata, imported documents |
| Context | the live window: Task Capsule, delegation capsule, compaction, session hook output |
| Memory | `project-brain/` records and `memory-bank/` chunks that outlive the window |
| Tool | hooks, `context.py`, wrappers - the layer that carries a call out and an observation back |
| Local environment | shell, filesystem, PHP runtime, the project's own tooling |
| External environment | GitHub, Packagist, remote APIs, the model host |

Two rules keep the label reproducible:

- **Earliest unrecovered failure.** When errors cascade, label the earliest failure after which the run never recovered, not the last symptom. Later errors are consequences, and a rule written against a consequence fixes nothing.
- **Blame follows behavior, not opportunity.** Blame the model when a more careful run under the same conditions would have avoided the failure or recovered from it. When the condition was genuinely unrecoverable - the service was down, the file was never delivered, the hook denied the call - blame the component that produced it, even if the model's fallback was also weak.

Most failures do land on the model side. The value of localizing is not the common case; it is reliably catching the minority that no rule for the model can fix.

## Routing

The blamed side decides which repair is legitimate:

| Blame | Repair belongs in | Anti-pattern |
|-------|-------------------|--------------|
| Model | skill instruction, `AGENTS.md` rule, review checklist | - |
| Context, Memory, or Tool | a hook, `context.py`, the capsule contract, a record template | a MUST rule asking the model to compensate for a harness defect it cannot observe |
| Local or external environment | a `project-brain/` incident; where the condition was recoverable, an explicit retry/fallback expectation for the model | any stabilization rule at all - nothing in this repository caused it |
| Owner or Grader | `DOD.md`, `AGENTS.md`, or the request; report the mismatch to the user | silently picking one side when an instruction and a check contradict each other |

## When To Stabilize

- The same mistake happens more than once.
- A user correction reveals a missing rule.
- A hook blocks too much or too little.
- A Laravel convention (Form Request validation, Policy authorization, Eloquent parameter binding) is violated repeatedly.
- A workflow handoff is confusing.

Use stabilization for enforceable behavior that prevents a repeated agent failure. Use `project-brain/` for governed active tasks, handoffs, findings, bugs, incidents, decisions, events, and promotion proposals. Use `memory-bank/` for verified governed reusable consequences and governed promotion application. Use `specs/` for authoritative architecture and behavioral contracts; canonical sources always outrank both context stores.

Session hooks may surface only mode, index health/staleness, active binding count, and validation status. They must never index, retrieve, print, or inject Project Brain or Memory Bank records automatically.

## Rule Template

```markdown
### Rule: [Short Name]

**Trigger:** [What happened]
**Edge:** [COMPONENT - COMPONENT]
**Blame:** [Responsible component]
**Root cause:** [Why it happened]
**Rule:** MUST/MUST NOT [enforceable behavior]
**Example:**
- Incorrect: [bad example]
- Correct: [good example]
**Enforcement:** Policy / skill instruction / hook / review checklist
**Added:** YYYY-MM-DD
```

`Edge` and `Blame` are not decoration: they must agree with the repair named in `Enforcement`. A rule blamed on Context, Memory, or Tool that is enforced only by a policy sentence is a localization error - fix the harness instead.

## Localization Examples

### Rule: Compaction Reports What It Dropped

**Trigger:** A Task Capsule compacted to fit its budget rendered as a clean summary. The constraint that justified an earlier decision was gone, and the next turn reversed that decision as an "optimization".
**Edge:** CONTEXT - MODEL
**Blame:** CONTEXT
**Root cause:** Missing enforcement. `enforce_capsule_budget` counted what it dropped, but nothing rendered the counters, so the agent could not tell a complete capsule from a lossy one.
**Rule:** MUST render a compaction line whenever the capsule drops or truncates content, and MUST re-read the cited source before revising an earlier decision from a compacted summary.
**Example:**
- Incorrect: treat a compacted capsule as the full record of the task's constraints.
- Correct: read the `compaction:` line, then open the cited source before changing a decision it may no longer explain.
**Enforcement:** `memory-bank/scripts/context.py` (`print_capsule`), `AGENTS.md`.
**Added:** 2026-08-15

### Case Without A Rule: Registry Timeout

`composer audit` fails because Packagist times out.

- If the timeout was transient and the run gave up without retrying, without trying another route, and without reporting the check as unrun: `EXTERNAL ENVIRONMENT - MODEL | blame: MODEL`. That yields a rule about recovery and honest reporting.
- If the registry was genuinely unreachable: `EXTERNAL ENVIRONMENT - MODEL | blame: EXTERNAL ENVIRONMENT`. That yields a `project-brain/` incident and nothing else. A rule forbidding Packagist to time out is unenforceable noise, and adding one trains the roster to ignore the rest.

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
