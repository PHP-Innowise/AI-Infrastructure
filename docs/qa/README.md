# QA Artifact Tooling

This directory is the working output root for the 75-run remediation. It does
not contain a fabricated disposition ledger.

Both builders require every primary file listed in `provenance.json`, verify
the package checksum manifest, and fail before writing output if any source is
missing or inconsistent:

```bash
python3 scripts/qa/build_disposition_ledger.py
python3 scripts/qa/reconstruct_workbook.py
```

Install the workbook dependency in an isolated environment:

```bash
python3 -m venv /tmp/accelerator-qa-venv
/tmp/accelerator-qa-venv/bin/python -m pip install -r requirements-qa.txt
```

List or run the deterministic cases:

```bash
python3 scripts/qa/run_tc_ai.py --list
python3 scripts/qa/run_tc_ai.py --run-id RUN-20260803-001
```

Evidence is written under `evidence/<Run ID>/`. Existing run directories are
never overwritten. Hybrid or native cases remain `skipped` when native-client
evidence or a focused regression is unavailable.

After a complete evidence run, apply the reviewed owner/disposition policy and
bind reproduced Codex evidence into the working ledger:

```bash
python3 scripts/qa/apply_disposition_decisions.py
```

The reproducible policy is recorded in `disposition-decisions.json`.
`evidence-backed-local-v1` approves only checksum-backed historical Pass records
without a recorded defect and newly reproduced Codex runs whose complete
schema-validated evidence, catalog identity, ancillary documents, assertion
outcomes, command exit codes, and checksum manifest consistently establish
status `passed`.
Historical non-passes, defect-bearing historical Passes, skipped/failed reruns,
unavailable tools, native-client observations, and external client findings
remain `review_status: pending`.
Changing this scope requires a reviewed policy edit rather than a blanket
status replacement.

Validate working artifacts:

```bash
python3 scripts/qa/validate_qa_artifacts.py
```

Use `--release` only after all 75 dispositions, all 18 run-evidence
directories, owners, reviews, evidence links, and checksums are complete.
