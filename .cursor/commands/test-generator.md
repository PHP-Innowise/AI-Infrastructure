---
name: test-generator
description: "Spawn test-generator agent to create comprehensive Pest/PHPUnit tests for Laravel routes, Form Requests, Policies, and Eloquent models."
---

# Test Generator

Spawn test-generator agent to create comprehensive Pest/PHPUnit tests for Laravel routes, Form Requests, Policies, and Eloquent models.

## Input
$ARGUMENTS

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `test-generator`
- **description:** `Generate tests`
- **prompt:** `$ARGUMENTS`

The agent will use the test-generator skill and suggest next steps when done.
