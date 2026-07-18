---
spawns: dependency-manager-agent
phase: execution
flow-next: verify
flow-alternatives: [security-reviewer, researcher, code-reviewer]
---

# Dependency Manager

Spawn dependency-manager agent to audit, update, and vet Composer dependencies for a Laravel project (composer audit, outdated review, Laravel version compatibility, autoload optimization, package vetting).

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `dependency-manager`
- **description:** `Manage dependencies`
- **prompt:** `$ARGUMENTS`

The agent will use the dependency-manager skill and suggest next steps when done.
