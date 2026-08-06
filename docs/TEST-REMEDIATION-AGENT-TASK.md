# Accelerator Test Evidence Collection — AI Agent Task

## Purpose

Use this document with the AI agent in the environment where accelerator tests
were originally executed.

The agent's task is **evidence collection only**. It must not fix accelerator
code, redesign tests, update the workbook, or make governance decisions.

The central team can implement all known source and test fixes locally. What is
missing from several historical runs is independently reviewable evidence:

- original logs;
- exact commands and exit codes;
- native Claude Code, Cursor, or Codex behavior;
- screenshots or sanitized transcripts;
- environment and version information;
- clarification of missing or ambiguous test definitions.

Each tester should use this task to collect everything available for their Run
IDs into one report and send that report to the central team.

Required output:

```text
ACCELERATOR-TEST-EVIDENCE-REPORT.md
```

Optional raw logs and screenshots may accompany the report, but the report must
contain enough information to understand every attachment without opening it.

---

## Short Instructions for the Tester

1. Open the repository and AI tool used during your testing.
2. Give the AI agent this file and `Accelerator TestCases.xlsx`.
3. Specify all Run IDs that you executed.
4. Give the agent access to any surviving logs, screenshots, transcripts, or
   temporary test projects.
5. Ask the agent to follow this file and create one evidence report.
6. Review the report for secrets or private data.
7. Send the report and referenced safe attachments to the central team.

Use this assignment prompt:

```text
Collect evidence for the accelerator tests I executed.

Assigned Run IDs:
<INSERT ALL RUN IDS>

Repository path:
<INSERT PATH>

Workbook path:
Accelerator TestCases.xlsx

Original evidence locations, if known:
<INSERT PATHS OR "unknown">

Follow docs/TEST-REMEDIATION-AGENT-TASK.md completely.

Create one file named ACCELERATOR-TEST-EVIDENCE-REPORT.md.
Do not implement fixes or modify accelerator source files.
Do not stage, commit, or push anything.
```

One agent may process all Run IDs executed by the same tester. There is no need
to launch a separate agent for every run.

---

## Required Final Deliverable

Create:

```text
ACCELERATOR-TEST-EVIDENCE-REPORT.md
```

The report must contain:

1. tester and environment information;
2. one complete section per assigned Run ID;
3. historical evidence that still exists;
4. new reproduction or native-client evidence when safely possible;
5. missing evidence explicitly identified;
6. test-definition clarifications;
7. a consolidated evidence inventory;
8. a final list of files sent to the central team.

Do not create a remediation plan. Do not edit product code. Do not declare that
a defect is fixed.

---

## Safety and Privacy Rules

The agent must:

- treat the repository as read-only, except for the final report and explicitly
  approved evidence files;
- never run `git add`, commit, push, reset, clean, restore, or discard changes;
- never modify accelerator source files;
- never edit `Accelerator TestCases.xlsx`;
- never read, print, copy, or attach `.env` contents;
- never collect credentials, tokens, private keys, cookies, production data,
  customer data, or confidential logs;
- never access a real production database;
- never include raw prompts or transcripts containing sensitive information;
- sanitize paths that expose private usernames when sharing externally;
- use temporary synthetic projects for new reproductions;
- never install AI clients, enable telemetry, or change credentials without
  explicit approval;
- never invent missing logs, commands, results, timestamps, or test
  expectations;
- mark unavailable evidence as unavailable.

If a historical artifact contains sensitive content, do not attach it. Record:

```text
Artifact exists but was withheld because it contains sensitive information.
Sanitized replacement: <path or unavailable>
```

---

## Evidence Status Vocabulary

Use exactly one status for every expected artifact:

- `Available — original`
- `Available — newly reproduced`
- `Available — sanitized`
- `Unavailable — deleted`
- `Unavailable — location unknown`
- `Unavailable — environment no longer exists`
- `Unavailable — required AI client missing`
- `Withheld — sensitive`
- `Not Applicable`

Do not use `Pass` as an evidence status.

---

## Evidence Confidence

Assign one confidence level to every run:

- `High` — original logs or immutable CI evidence, exact commit, and complete
  commands are available;
- `Medium` — behavior was newly reproduced on the same commit or a clearly
  documented equivalent environment;
- `Low` — only workbook text, memory, or static source inspection is available;
- `Insufficient` — the run cannot be independently reviewed.

Explain the confidence rating.

---

## Required Workflow

### 1. Identify the tester's assigned runs

Read the workbook and list every assigned:

- Run ID;
- Test Case ID;
- recorded result;
- framework;
- AI tool;
- accelerator version;
- Git commit;
- execution date;
- evidence link;
- Defect ID, if present.

The workbook reuses `DEF-001` through `DEF-004` for unrelated defects. Identify
a defect using:

```text
Run ID + Test Case ID + Defect ID
```

Do not join records by Defect ID alone.

### 2. Record the original environment

For each distinct environment, record:

- OS and version;
- shell;
- PHP version;
- Python version, if used;
- framework and version;
- AI tool and version;
- model name, if known;
- accelerator version;
- exact Git commit;
- operating mode;
- project size classification;
- whether the repository was clean;
- whether execution used the real repository, a clone, or a temporary fixture;
- whether the AI client was genuinely running or behavior was scripted.

If a value is unknown, write `Unknown`. Do not infer it from unrelated runs.

### 3. Search for original evidence

Search only approved locations:

- evidence directories created by the tester;
- CI artifacts;
- terminal logs;
- screenshots;
- sanitized AI-client transcripts;
- temporary fixture repositories that still exist;
- exported test reports;
- test runner output.

For every artifact, record:

- artifact name;
- Run ID;
- absolute local path;
- safe shareable relative name;
- size;
- capture timestamp, if available;
- SHA-256 checksum;
- whether it contains sensitive material;
- whether it will be sent.

Do not modify the original artifact.

### 4. Validate historical evidence

For each original log:

- confirm that it references the expected Run ID or Test Case ID;
- confirm the repository commit when possible;
- identify exact commands;
- record exit codes;
- identify passed, failed, skipped, or unexecuted assertions;
- compare the log with the workbook's Actual Behaviour;
- record contradictions without rewriting either source.

Do not state that a log proves native-client behavior if it contains only static
inspection or direct script execution.

### 5. Clarify the test definition

The workbook contains only 19 Test Case definitions for 67 run-referenced Test
Case IDs.

For every assigned Test Case ID missing from the `Test Cases` sheet, provide:

- original test title, if known;
- original objective;
- preconditions;
- exact input or a sanitized equivalent;
- mandatory assertions;
- optional assertions;
- scriptable assertions;
- native-client-only assertions;
- expected files or state changes;
- expected safety behavior;
- source document or requirement;
- who authored or approved the definition.

Label the definition:

- `Authoritative — original source found`;
- `Reconstructed — tester confirmed`;
- `Derived — workbook evidence only`;
- `Unknown`.

Do not describe a reconstructed definition as authoritative.

### 6. Reproduce only when useful and safe

A new reproduction is useful when:

- the original log is missing;
- the test depended on native AI-client behavior;
- the workbook result is ambiguous;
- the original environment can be recreated safely;
- a synthetic equivalent can demonstrate the same behavior.

For new reproductions:

- use a temporary synthetic project;
- record the current commit separately from the historical commit;
- do not claim that current behavior proves the historical result;
- capture exact commands and exit codes;
- retain logs in a stable evidence directory;
- record all retries;
- distinguish agent behavior from direct script execution;
- report any repository mutations.

Stop after collecting evidence. Do not implement the fix.

### 7. Capture native AI-client evidence

When the test involves Claude Code, Cursor, or Codex, record:

- client and version;
- model;
- relevant settings;
- whether the repository was trusted;
- which skills, commands, agents, rules, or hooks were discovered;
- the sanitized synthetic prompt;
- expected behavior;
- observed behavior;
- whether a real command → agent → skill path executed;
- relevant hook events;
- safe transcript excerpts;
- screenshots when they materially prove host behavior;
- files created or modified;
- whether the behavior can be reproduced.

A source file saying that a command should spawn an agent is static evidence.
It is not proof that the native client actually spawned the agent.

### 8. Produce one consolidated report

Use the report template in this document. Include all assigned Run IDs in the
same file.

---

## Priority Evidence Needed from Historical Runs

The central team can implement product fixes locally. The following evidence is
especially valuable because it cannot be recovered reliably from source code
alone.

### Native cross-tool execution

Related run:

```text
RUN-20260803-018
```

Collect:

- Claude Code availability and version;
- Cursor availability and version;
- Codex availability and version;
- activation and discovery behavior;
- native hook execution;
- trust or permission behavior;
- exact reason an unavailable client could not run.

### Infrastructure-Creator generation

Related run:

```text
RUN-006 / TC-007
```

Collect evidence that a live AI agent:

- consumed the Project Profile;
- ran the required forge workflow;
- generated only selected editions;
- produced the generation report;
- invoked structural validation;
- preserved unrelated target files.

### Infrastructure-Creator collision interaction

Related run:

```text
RUN-007 / TC-008
```

Collect:

- collision detection output;
- alternatives shown to the user;
- user's selected decision;
- files considered team-owned;
- proof that existing files remained unchanged;
- resulting manifest ownership.

Do not fix the known placeholder-validator problem in this task.

### Infrastructure-Creator legacy update refusal

Related run:

```text
RUN-009 / TC-015
```

Collect evidence that the live agent:

- detected the missing manifest;
- explained why update was unsafe;
- offered supported recovery options;
- refused pressure to update unsafely;
- performed zero target writes.

### Hook generation

Related run:

```text
RUN-010 / TC-016
```

Collect evidence that the live agent:

- derived hook rules from Project Profile evidence;
- cited the relevant profile sections;
- produced `hook-forge-log.md`;
- generated correct Claude/Cursor host-specific wiring;
- preserved Cursor's read-path asymmetry.

### Generic PHP memory generation

Related run:

```text
RUN-011 / TC-017
```

Collect evidence that the live agent:

- recognized a framework-less PHP project;
- selected `generic` instead of inventing a framework;
- recorded the reason;
- generated valid Memory Bank and Project Brain assets;
- selected the correct canonical edition.

### Security-review agent behavior

Related run:

```text
RUN-015 / TC-032
```

Using a synthetic vulnerable project, collect evidence that:

- the real security-review command invoked the expected agent and skill;
- planted vulnerabilities were detected;
- findings used the required severity model;
- `.env` and real secrets were not read or exposed;
- the final verdict matched discovered severity;
- documented next-step alternatives matched actual behavior.

### Governed checkpoint and memory commands

Related run:

```text
RUN-049 / TC-072
```

Collect evidence that:

- argument-bearing checkpoint invocation refuses safely;
- argument-free governed checkpoint does not create competing SQLite authority;
- memory validates and refreshes without changing task lifecycle;
- no completion or promotion occurs;
- reported layers match CLI state.

### Profile synthesis

Related run:

```text
RUN-025 / TC-048
```

Collect:

- all required scanner findings;
- research findings;
- clarifying-interview answers;
- synthesis invocation;
- resulting Project Profile;
- confidence and source-type handling;
- contradictions and unresolved questions;
- proof that target files were not generated during profile synthesis.

### Full infrastructure scan

Related run:

```text
RUN-033 / TC-056
```

Collect:

- scanner fan-out;
- scanner completion status;
- research output;
- interview questions and sanitized answers;
- synthesis result;
- checkpoint behavior;
- target repository mutation status.

### Stack adaptation

Related run:

```text
RUN-034 / TC-057
```

Collect:

- detected non-PHP stack;
- explicit consent;
- ecosystem research;
- sibling generator location;
- authored skills and edition mirrors;
- copied stack-agnostic assets;
- self-verification output;
- proof that the source generator was not modified.

### One-shot infrastructure build

Related run:

```text
RUN-035 / TC-058
```

Collect:

- scan invocation;
- profile checkpoint decision;
- ambiguity or collision gate;
- generation invocation;
- selected editions;
- validation result;
- generated manifest;
- final build report;
- source and target repository cleanliness.

---

## Evidence for Source-Backed Findings

The following findings can be fixed locally by the central team, but original
evidence is still useful for traceability.

### Automatic promotion policy conflict

```text
RUN-20260803-011 / TC-AI-011
```

Collect:

- exact reproduction command;
- runtime configuration;
- created Project Brain record;
- promotion proposal/review/application records;
- resulting Memory Bank chunk metadata;
- proof of `reviewer: null` or equivalent behavior;
- test output and exit code;
- policy source cited by the tester.

### Automatic completion policy conflict

```text
RUN-20260803-012 / TC-AI-012
```

Collect:

- task state before merge detection;
- merge-boundary trigger;
- lifecycle transitions;
- local episode creation, if any;
- task state after maintenance;
- policy source cited by the tester;
- test output and exit code.

### Infrastructure-Creator integration coverage gap

```text
RUN-20260803-014 / TC-AI-014
```

Collect:

- exact test discovery command;
- discovered test list;
- 21-hook-test output;
- proxy-run commands and 34 diagnostics;
- explanation of why the proxy was not a valid generated target;
- any abandoned or external end-to-end harness.

### Retrieval quality benchmark

```text
RUN-20260803-016 / TC-AI-016
```

This evidence is particularly important because the original manifests were
ephemeral.

Collect:

- all four original queries;
- complete candidate/result paths;
- top-five rankings per query;
- relevance judgments;
- precision and recall calculation;
- token estimates;
- latency measurements;
- corpus commit;
- Cursor-specific weak results;
- original retrieval manifests, if they still exist.

### Cursor first-turn continuity

```text
RUN-004 / TC-004
```

Collect:

- first through fifth Stop hook outputs;
- task-binding state after each turn;
- presence or absence of `working-memory.mdc`;
- live Cursor proof that the rule was or was not attached to the next prompt;
- timeout values and environment variables;
- hook configuration.

### Unowned team `AGENTS.md`

```text
RUN-007 / TC-008
```

Collect:

- pre-existing `AGENTS.md` checksum;
- manifest contents;
- exact placeholder text;
- validator command and output;
- post-run checksum;
- proof that the file was not manifest-owned.

### Team files under generated roots

```text
RUN-008 / TC-014
```

Collect:

- full manifest before update;
- team-added file and checksum;
- update decision records;
- validator command and output;
- proof that the file was neither claimed nor modified;
- exact generated-file-not-tracked diagnostic.

### Bash validator minimal environment

```text
RUN-014 / TC-030
```

Collect:

- exact JSON input;
- exact PATH;
- command invocation;
- stdout;
- stderr;
- exit code;
- extractor availability;
- retry reason and corrected invocation.

### Refresh privacy behavior

```text
RUN-043 / TC-066
```

Do not include real private information. Use synthetic values.

Collect:

- synthetic private-looking query;
- `retrieve`, `context`, and `refresh` results;
- manifest content after safe redaction;
- proof of tokenized fragments;
- file location and Git-ignore status;
- relevant test output.

### Handoff test-definition mismatch

```text
RUN-038 / TC-061
```

Collect:

- original case definition;
- source of the `implementing` phase expectation;
- exact failing command;
- supported CLI phases;
- original retrieval query and ranked results;
- handoff-focused query and results;
- tester's confirmation of intended behavior.

---

## Report Template

The AI agent must create one file using this structure:

```markdown
# Accelerator Test Evidence Report

## 1. Submission

- Tester:
- Team:
- Report created:
- Repository:
- Workbook:
- Assigned Run IDs:
- Evidence root:
- AI agent/tool used to collect evidence:

## 2. Environment Inventory

### Environment E-01

- OS:
- Shell:
- PHP:
- Python:
- Framework:
- AI client:
- AI client version:
- Model:
- Accelerator version:
- Git commit:
- Operating mode:
- Repository state:
- Real client or scripted execution:

## 3. Executive Evidence Summary

- Runs assigned:
- Original evidence available:
- Newly reproduced:
- Evidence unavailable:
- Evidence withheld:
- Native-client runs completed:
- Runs with contradictory records:
- Runs with missing Test Case definitions:

## 4. Run Evidence

### <RUN-ID> / <TEST-CASE-ID>

#### Workbook Record

- Recorded result:
- Framework:
- AI tool:
- Accelerator version:
- Git commit:
- Execution date:
- Defect reference:
- Workbook evidence link:

#### Test Definition

- Definition status:
- Title:
- Objective:
- Preconditions:
- Input or sanitized equivalent:
- Mandatory assertions:
- Optional assertions:
- Scriptable assertions:
- Native-client assertions:
- Expected state changes:
- Safety expectations:
- Requirement/source:

#### Historical Evidence

- Evidence status:
- Original artifacts:
- Commands:
- Exit codes:
- Passed assertions:
- Failed assertions:
- Skipped/unexecuted assertions:
- Relevant output excerpts:
- Contradictions with workbook:

#### New Reproduction

- Performed:
- Reason:
- Reproduction commit:
- Synthetic fixture:
- Exact commands:
- Exit codes:
- Observed behavior:
- Files created or modified:
- Source repository remained unchanged:

#### Native-Client Evidence

- Client and version:
- Model:
- Trust/settings:
- Sanitized task:
- Skills/commands/agents/hooks discovered:
- Observed orchestration:
- Safe transcript excerpts:
- Screenshots:
- Native assertions passed:
- Native assertions failed:
- Native assertions unavailable:

#### Evidence Assessment

- Primary classification:
- Confidence:
- Evidence proves:
- Evidence does not prove:
- Missing information:
- Tester clarification:
- Recommended central-team follow-up:

#### Artifact Inventory

- Artifact:
- Shareable filename:
- Status:
- SHA-256:
- Timestamp:
- Sensitive-content review:
- Included in submission:

## 5. Missing or Ambiguous Test Definitions

### <TEST-CASE-ID>

- Status:
- Reconstructed definition:
- Source:
- Tester confirmation:
- Remaining ambiguity:

## 6. Contradictions and Corrections Proposed

- Run ID:
- Workbook statement:
- Evidence statement:
- Proposed correction:
- Historical record must remain preserved:

## 7. Consolidated Artifact Inventory

- Artifact:
- Run IDs:
- Type:
- Status:
- SHA-256:
- Included:

## 8. Unavailable Evidence

- Run ID:
- Missing artifact:
- Last known location:
- Reason unavailable:
- Can it be reproduced:

## 9. Submission Checklist

- [ ] Every assigned Run ID has a report section
- [ ] Exact commits are recorded
- [ ] Original and new evidence are distinguished
- [ ] Static and native-client evidence are distinguished
- [ ] Missing artifacts are explicit
- [ ] No secrets or private data are included
- [ ] Checksums are recorded
- [ ] All attachments are referenced
- [ ] Accelerator source files were not modified
- [ ] Workbook was not edited
- [ ] Nothing was staged, committed, or pushed

## 10. Files Sent to the Central Team

- ACCELERATOR-TEST-EVIDENCE-REPORT.md
- <safe attachment>
- <safe attachment>
```

Repeat the `Run Evidence` section for every assigned Run ID.

---

## Optional Evidence Package

If raw logs or screenshots are needed, package them as:

```text
Accelerator-TestEvidence-Submission/
├── ACCELERATOR-TEST-EVIDENCE-REPORT.md
├── logs/
│   └── <RUN-ID>.log
├── screenshots/
│   └── <RUN-ID>-<description>.png
├── transcripts/
│   └── <RUN-ID>-sanitized.md
└── manifests/
    └── <RUN-ID>-<artifact>.json
```

The report remains the primary deliverable. Attachments are supporting
evidence, not replacements for report sections.

Do not include:

- `.env`;
- databases;
- credentials;
- raw customer data;
- full private prompts;
- unredacted transcripts;
- repository archives containing secrets;
- dependency caches;
- `vendor/`;
- local AI-client credential stores.

---

## Completion Criteria

This evidence-collection task is complete when:

1. every assigned Run ID has a report section;
2. every original artifact is inventoried;
3. missing evidence is explicitly recorded;
4. exact environments and commits are documented;
5. static, scripted, and native-client evidence are separated;
6. missing Test Case definitions are clarified where the tester has knowledge;
7. safe reproductions are documented without source changes;
8. attachments have checksums and are referenced from the report;
9. the submission contains no secrets or private data;
10. one consolidated report is ready to send to the central team.

The central team will use the report to implement fixes, update test
definitions, rerun regressions, and correct the final workbook. Testers and
their evidence-collection agents are not expected to implement those fixes.
