# User Task Workflow Example

This example shows what happens when a user starts and completes a task with the combined Project Brain, Local Context Engine, and Memory Bank.

## Roles in This Example

### Project Brain — What Is Happening Now

Project Brain is the shared active-work layer. It owns:

- the current task and its lifecycle;
- progress, next actions, and affected files;
- handoffs between agents or sessions;
- findings, bugs, incidents, decisions, and events;
- revisions, ownership, evidence, conflicts, and source fingerprints.

In this example, Project Brain creates and maintains `TASK-123`. It allows another agent to continue the task without the user repeating its history.

### Local Context Engine — How Relevant Information Is Found

The Local Context Engine is the local search and context-assembly layer. It:

- indexes eligible policies, skills, documentation, Memory Bank chunks, Project Brain records, and handoffs;
- searches them through SQLite FTS5/BM25;
- filters private, unauthorized, stale, ignored, superseded, or invalid content;
- returns bounded snippets within token budgets;
- records retrieval metadata in manifests.

Its SQLite database is disposable and non-authoritative. Deleting it removes the local index, but not Project Brain records, Memory Bank knowledge, or canonical project sources.

### Memory Bank — What the Project Has Learned Permanently

Memory Bank is the reviewed long-term knowledge layer. It stores stable, reusable project knowledge such as:

- business rules and verified constraints;
- architecture decisions and conventions;
- integration contracts;
- operational lessons;
- reusable consequences discovered during completed work.

Memory Bank does not store active task progress. In this example, the two-factor-authentication rule enters Memory Bank only after the task produces evidence and a human approves its promotion.

### Relationship Between the Three

```text
Project Brain
  tracks TASK-123 and its current progress
        ↓
Local Context Engine
  finds relevant Brain, Memory Bank, policy, and project-source snippets
        ↓
Agent
  verifies canonical sources and performs the work
        ↓
Memory Bank
  receives only reviewed reusable knowledge after completion
```

Canonical policy, specifications, code, configuration, migrations, and tests always remain the highest authority.

### Simple Mental Model for `TASK-123`

```text
Project Brain:
“TASK-123 is active; this is completed; this is next.”

Local Context Engine:
“Here are the most relevant searchable sources for TASK-123.”

Memory Bank:
“This verified 2FA rule should be remembered permanently.”
```

## 1. User Starts the Task

The user writes:

> Let’s implement TASK-123: add two-factor authentication.
> Requirements are in `specs/two-factor-auth.md`.

A stable task ID should be provided for governed work. If it is missing, the agent asks the user for one instead of deriving an authoritative task from the Git branch.

## 2. Agent Selects the Appropriate Workflow

The agent:

1. reads the project policy and relevant skill;
2. identifies the request as non-trivial implementation work;
3. checks `project-brain/config/runtime.json`;
4. uses governed mode by default;
5. checks whether `TASK-123` already exists.

If the task exists, the agent resumes it. Otherwise, it starts a new task.

## 3. Project Brain Creates the Task

The agent internally runs:

```bash
python3 memory-bank/scripts/context.py start \
  --task-id TASK-123 \
  --goal "Add two-factor authentication" \
  --source specs/two-factor-auth.md
```

This creates:

```text
project-brain/dynamic/tasks/<task-uuid>.md
project-brain/control/handoffs/<task-uuid>.md
```

The task record contains:

- goal and current progress;
- owner and authorized owners;
- lifecycle status and revision;
- affected files and source references;
- privacy, authority, and confidence;
- source fingerprints and conflicts;
- append-only transition history.

SQLite stores only the local binding between `TASK-123` and the Project Brain UUID. It does not store a second authoritative progress record.

## 4. Agent Refreshes Searchable Context

The agent refreshes the local index:

```bash
python3 memory-bank/scripts/context.py index --json
```

The Local Context Engine indexes eligible:

- policies and skills;
- project documentation and specifications;
- active Memory Bank chunks;
- Project Brain records and handoffs;
- relevant historical episodes.

Before reading or indexing content, it excludes ignored, sensitive, unauthorized, private, stale, superseded, terminal, or invalid files.

## 5. Agent Retrieves Relevant Context

The agent runs:

```bash
python3 memory-bank/scripts/context.py retrieve \
  "two-factor authentication requirements and security rules" \
  --task-id TASK-123
```

The engine:

1. finds candidates with SQLite FTS5/BM25;
2. applies privacy, ownership, authority, lifecycle, and freshness filters;
3. returns bounded snippets instead of complete documents;
4. keeps relevant conflicting records together;
5. applies token budgets;
6. creates a retrieval manifest.

Example manifest:

```text
project-brain/control/retrieval-manifests/<manifest-uuid>.json
```

The manifest records what was selected or excluded, but never stores prompts, responses, source bodies, or hidden reasoning.

## 6. Agent Verifies Canonical Sources

The agent opens the actual referenced files, for example:

```text
specs/two-factor-auth.md
src/Security/AuthenticationService.php
config/security.php
tests/Security/AuthenticationTest.php
```

Retrieved context is only a discovery aid. Current policy, specifications, code, configuration, migrations, and tests remain authoritative.

## 7. Agent Implements the Task

The implementation skill performs the requested work:

1. updates application code;
2. adds migrations or configuration when required;
3. creates or updates tests;
4. follows framework architecture and security rules;
5. runs relevant verification.

Project Brain manages task continuity; it does not replace implementation, architecture, debugging, or testing skills.

## 8. Agent Records Progress

After a meaningful milestone, the agent runs:

```bash
python3 memory-bank/scripts/context.py update \
  --task-id TASK-123 \
  --revision 1 \
  --progress "TOTP enrollment and verification implemented" \
  --next-step "Implement recovery codes" \
  --file src/Security/TwoFactorService.php \
  --source specs/two-factor-auth.md
```

The engine:

- rejects stale revisions;
- verifies owner authorization;
- updates the shared task;
- refreshes its handoff;
- updates the local revision binding;
- rolls back both stores if one update fails.

Only a concise progress summary is stored. Raw diffs, conversations, prompts, responses, logs, and secrets are prohibited.

## 9. Session Stops Before Completion

The task remains active. The handoff might contain:

```text
Goal: Add two-factor authentication.
Current state: TOTP enrollment and verification are implemented.
Next action: Implement and test recovery codes.
Relevant files:
- src/Security/TwoFactorService.php
- tests/Security/TwoFactorServiceTest.php
```

The handoff is a continuation summary, not a transcript.

## 10. User Continues Later

In a new session, the user writes:

> Continue TASK-123.

The next agent:

1. loads the existing Project Brain task;
2. reads its current revision and handoff;
3. refreshes the local index;
4. retrieves fresh task-specific context;
5. verifies the referenced canonical sources;
6. continues from the recorded next action.

The user does not need to repeat the complete task history.

## 11. Agent Verifies the Result

Before completion, the agent runs applicable checks:

```text
tests
formatting
static analysis
framework validation
Memory Bank validation
Project Brain validation
```

If verification fails, the task remains active, blocked, or verifying. It is not falsely marked completed.

## 12. Agent Completes the Task

After successful verification:

```bash
python3 memory-bank/scripts/context.py complete \
  --task-id TASK-123 \
  --revision 3 \
  --outcome "Two-factor authentication implemented" \
  --verification "Authentication and recovery-code tests passed"
```

The engine:

1. advances the Project Brain task to `completed`;
2. closes its handoff;
3. removes the active SQLite binding;
4. may store a non-authoritative local replay episode.

## 13. Agent Proposes Reusable Knowledge

The task may produce a reusable project rule:

> Administrative users must complete TOTP enrollment before accessing protected administration routes.

The agent may create a Memory Bank promotion proposal referencing the exact Project Brain source IDs and revisions.

The agent cannot approve its own proposal.

## 14. User Reviews the Promotion

The user approves or rejects the proposal.

After independent human approval, the knowledge is written to:

```text
memory-bank/chunks/MEM-XXXX-two-factor-authentication-policy.md
```

The Memory Bank index and counter are updated together. If validation fails, all promotion writes are rolled back.

## 15. Completed Work Is Archived

Later, the agent can compact Project Brain:

```bash
python3 memory-bank/scripts/context.py compact
```

Completed or superseded records and related handoffs move to the archive. They are not deleted.

## What the User Normally Does

The normal user interaction is simple:

```text
User: Let’s implement TASK-123: add two-factor authentication.
Agent: Starts or resumes the governed task, retrieves context, implements,
       verifies, updates the handoff, and reports progress.

User: Continue TASK-123.
Agent: Loads the handoff and continues from the recorded next action.

User: Approve the proposed reusable authentication rule.
Agent: Applies the reviewed promotion to the durable Memory Bank.
```

The CLI commands are normally executed by the agent under accelerator policy. Users may invoke `/memory`, `/checkpoint`, `/project-brain`, or `/memory-bank` directly when they want explicit control.

No hook automatically reads records, indexes sources, or injects context into a prompt.
