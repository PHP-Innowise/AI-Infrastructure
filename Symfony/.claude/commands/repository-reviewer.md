---
spawns: repository-reviewer-agent
phase: quality
flow-next: code-reviewer
flow-alternatives: [performance-optimization, database-designer, coder]
---

# Repository Reviewer

Review Doctrine repositories for query correctness, parameter binding, indexes, pagination, and N+1 risk.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn `repository-reviewer`.
