# tasks/

One `TASK-{N}/` directory per scan+generate run against a target, where `{N}` is a zero-padded number allocated from `.task-counter`.

## Convention

- Check `.task-counter` before creating a new task dir; create `TASK-{N}/`; increment the counter.
- Prefix every markdown file with the skill that produced it: `stack-scanner-findings.md`, `infra-scan-project-profile.md`, `infra-generate-report.md`, etc. (`README.md` is the only unprefixed file allowed here.)

## Typical Contents Of A Run

```
TASK-001/
├── stack-scanner-findings.md
├── architecture-scanner-findings.md
├── integration-scanner-findings.md
├── infra-ops-scanner-findings.md
├── security-compliance-scanner-findings.md
├── conventions-scanner-findings.md
├── stack-researcher-findings.md
├── clarifying-interview-questions.md
├── clarifying-interview-answers.md
├── infra-scan-project-profile.md      # the Phase 1 deliverable (review this)
├── infra-generate-report.md           # what Phase 2 wrote, where
└── bootstrap-verifier-report.md       # the QA gate result
```

These are the generator's own working documents. Nothing here is ever written into the target project.
