---
spawns: security-reviewer-agent
phase: execution
flow-next: verify
flow-alternatives: [coder, code-reviewer, debugger]
---

# Security Reviewer

Spawn security-reviewer agent to audit Laravel changes against the OWASP Top 10 (injection, XSS, mass assignment, access control, auth/session, CSRF, file upload, secrets, deserialization, SSRF) and run a dependency audit.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `security-reviewer`
- **description:** `Security audit`
- **prompt:** `$ARGUMENTS`

The agent will use the security-reviewer skill and suggest next steps when done.
