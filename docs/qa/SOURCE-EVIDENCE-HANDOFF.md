# Historical Source Evidence Handoff

The historical evidence package is intentionally excluded from Git. It may
contain client-sensitive material and must be transferred through an approved
private channel by the Accelerator Team.

## Required Location

Place the approved package at:

```text
docs/Accelerator-TestEvidence-Submission/
```

It must contain every path listed in `docs/qa/provenance.json`, including the
historical manifests, workbook snapshot, evidence report, and
`SHA256SUMS.txt`.

## Verification

Before using the package:

```bash
cd docs/Accelerator-TestEvidence-Submission
shasum -a 256 -c SHA256SUMS.txt
```

Do not continue if any included checksum fails. The intentionally withheld
confidential RUN-057 original cannot be independently verified from this
repository; use only the approved sanitized replacement supplied by the
External Client Team.

## Reconstruct the Historical Workbook

After installing `requirements-qa.txt` in the documented QA environment:

```bash
scripts/qa/.venv/bin/python scripts/qa/reconstruct_workbook.py \
  --output /tmp/Accelerator-TestCases.reconstructed.xlsx
```

For historical `RUN-001..RUN-057` assignments, give the tester or AI agent the
generated `/tmp/Accelerator-TestCases.reconstructed.xlsx`. Codex
`RUN-20260803-001..018` definitions come from `qa/tc-ai/cases.json`; the
committed source workbook is not required for normal TC-AI execution.

## Privacy

- Do not commit this package or generated evidence directories.
- Do not share raw client logs, secrets, tokens, `.env` values, personal
  information, hostnames, or machine-local paths.
- Sanitize reports, screenshots, transcripts, command output, and generated
  workbooks before returning them.
- Transfer confidential material only through the approved private channel.

The committed QA package is sufficient for deterministic TC-AI execution.
Historical reconstruction and native/external review require this separately
supplied source package.
