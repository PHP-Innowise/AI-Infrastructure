# QA Artifact Tooling

This directory is the working output root for the 75-run remediation. It does
not contain a fabricated disposition ledger.

Both builders require every primary file listed in `provenance.json`, verify
the package checksum manifest, and fail before writing output if any source is
missing or inconsistent:

```bash
scripts/qa/.venv/bin/python scripts/qa/build_disposition_ledger.py
scripts/qa/.venv/bin/python scripts/qa/reconstruct_workbook.py \
  --output /tmp/Accelerator-TestCases.reconstructed.xlsx
```

The required historical source package is intentionally not committed. Obtain
the approved sanitized package from the Accelerator Team and place it at
`docs/Accelerator-TestEvidence-Submission/` before running either builder.
Verify its `SHA256SUMS.txt` before use. See
[`SOURCE-EVIDENCE-HANDOFF.md`](SOURCE-EVIDENCE-HANDOFF.md).

Install the workbook dependency in an isolated environment. Use this path:
`python3 scripts/check.py` runs the `qa-tooling` job with
`scripts/qa/.venv/bin/python`, and skips it (naming these two commands) when
that venv is absent; Git ignores the folder.

```bash
python3 -m venv scripts/qa/.venv
scripts/qa/.venv/bin/python -m pip install -r requirements-qa.txt
```

List or run the deterministic cases:

```bash
scripts/qa/.venv/bin/python scripts/qa/run_tc_ai.py --list
scripts/qa/.venv/bin/python scripts/qa/run_tc_ai.py \
  --run-id RUN-20260803-001
```

Evidence is written under `evidence/<Run ID>/`. Existing run directories are
never overwritten. Hybrid or native cases remain `skipped` when native-client
evidence or a focused regression is unavailable.

After a complete evidence run, apply the reviewed owner/disposition policy and
bind reproduced Codex evidence into the working ledger:

```bash
scripts/qa/.venv/bin/python scripts/qa/apply_disposition_decisions.py
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
scripts/qa/.venv/bin/python scripts/qa/validate_qa_artifacts.py \
  --skip-run-evidence
```

For the release gate, obtain the approved private run evidence, pass its
explicit path, and add `--release`. Use release mode only after all 75
dispositions, all 18 run-evidence directories, owners, reviews, evidence links,
and checksums are complete.
