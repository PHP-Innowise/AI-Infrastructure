# QA Artifact Tooling

This directory is the working output root for the 75-run remediation. It does
not contain a fabricated disposition ledger.

Both builders require every primary file listed in `provenance.json`, verify
the package checksum manifest, and fail before writing output if any source is
missing or inconsistent:

```bash
infra-engine qa build-ledger
infra-engine qa reconstruct-workbook \
  --output /tmp/Accelerator-TestCases.reconstructed.xlsx
```

The required historical source package is intentionally not committed. Obtain
the approved sanitized package from the Accelerator Team and place it at
`docs/Accelerator-TestEvidence-Submission/` before running either builder.
Verify its `SHA256SUMS.txt` before use. See
[`SOURCE-EVIDENCE-HANDOFF.md`](SOURCE-EVIDENCE-HANDOFF.md).

Use the verified native `infra-engine` release from the repository root.
Workbook parsing, XML, checksums and schema validation run in Rust; no QA
virtualenv is required. The current native check catalog is
`install/check-catalog.json`; it has no `qa-tooling` group.

The historical TC-AI catalog uses schema `1.0.0`. The native runner can list
it, but refuses to execute its Python commands. Before running cases, review
an explicit mapping for each `run_id/command_id` that needs a native argv,
then migrate the active catalog while preserving the historical input:

```bash
infra-engine qa migrate-catalog --input qa/tc-ai/cases.json \
  --mappings /path/to/reviewed-native-commands.json \
  --output qa/tc-ai/cases.json --legacy-output qa/tc-ai/cases.v1.json
```

The migration accepts only a legacy catalog, refuses unused mappings and Python
commands, and emits schema `2.0.0`. It does not run or regrade historical cases.
Keep original reports and checksums as historical evidence.

List cases, or run the migrated native catalog:

```bash
infra-engine qa run --list
infra-engine qa run \
  --run-id RUN-20260803-001
```

Evidence is written under `evidence/<Run ID>/`. Existing run directories are
never overwritten. Hybrid or native cases remain `skipped` when native-client
evidence or a focused regression is unavailable.

After a complete evidence run, apply the reviewed owner/disposition policy and
bind reproduced Codex evidence into the working ledger:

```bash
infra-engine qa apply-decisions
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
infra-engine qa validate \
  --workbook /tmp/Accelerator-TestCases.reconstructed.xlsx --skip-run-evidence
```

For the release gate, obtain the approved private run evidence, pass its
explicit path, and add `--release`. Use release mode only after all 75
dispositions, all 18 run-evidence directories, owners, reviews, evidence links,
and checksums are complete.
