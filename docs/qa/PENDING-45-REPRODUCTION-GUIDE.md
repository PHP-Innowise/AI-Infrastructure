# Pending 45 Cases — Developer Testing Guide

## Task

Reproduce only the Run IDs assigned to you and return independently reviewable,
sanitized evidence. Do not modify accelerator source code or change QA
dispositions while testing.

Record the repository commit SHA, operating system, PHP/Python versions,
AI-client version, exact commands, exit codes, and unavailable dependencies.

## Pending Cases

Historical/native:

```text
RUN-001, RUN-002, RUN-003, RUN-004, RUN-005, RUN-006, RUN-007,
RUN-008, RUN-009, RUN-010, RUN-011, RUN-012, RUN-014, RUN-015,
RUN-025, RUN-027, RUN-029, RUN-030, RUN-031, RUN-033, RUN-034,
RUN-035, RUN-036, RUN-038, RUN-042, RUN-043, RUN-044, RUN-048,
RUN-049, RUN-050, RUN-051, RUN-052, RUN-053, RUN-054, RUN-055,
RUN-056, RUN-057
```

Codex:

```text
RUN-20260803-008, RUN-20260803-009, RUN-20260803-010,
RUN-20260803-011, RUN-20260803-012, RUN-20260803-014,
RUN-20260803-015, RUN-20260803-018
```

RUN-053, RUN-056, and RUN-057 require External Client Team verification.
RUN-057 must use only approved, sanitized evidence.

## Historical and Native Cases

Follow `docs/TEST-REMEDIATION-AGENT-TASK.md`. Use the AI client specified by
the test record and preserve:

- the objective, steps, expected behavior, and observed behavior;
- exact commands, outputs, and exit codes;
- environment and native-client versions;
- sanitized logs, transcripts, and screenshots;
- the final result: `passed`, `failed`, `skipped`, or `blocked`;
- a reason for every result other than `passed`.

Return an `ACCELERATOR-TEST-EVIDENCE-REPORT.md` describing all attachments.
Historical definitions and evidence must come from the separately supplied,
checksum-verified package described in
`docs/qa/SOURCE-EVIDENCE-HANDOFF.md`.

## Codex Cases

```bash
scripts/qa/.venv/bin/python scripts/qa/run_tc_ai.py --list

scripts/qa/.venv/bin/python scripts/qa/run_tc_ai.py \
  --run-id RUN-20260803-XXX \
  --output-root docs/qa/evidence-<developer>
```

Repeat `--run-id` for multiple assigned cases. Use a new output directory for
each clean attempt because the runner never overwrites existing evidence.

`--native-host codex` records host availability only. Native-client behavior
must still be captured and reviewed separately.

## Return

Return assigned Run IDs and outcomes, commit SHA, environment versions, exact
commands and exit codes, generated evidence directories, sanitized native
transcripts/screenshots, and explanations for all non-passing checks.

Never include secrets, tokens, `.env` values, customer data, personal
information, or original confidential RUN-057 material. Do not convert a
legitimate failed, skipped, or blocked result into a pass.
