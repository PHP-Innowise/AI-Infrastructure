---
name: test-generator
description: "Use this agent to generate Laravel tests following project patterns. Creates Pest or PHPUnit feature tests for routes, Form Request validation tests, Policy authorization tests, Eloquent model/factory-based tests, and tests using fakes for queues, mail, notifications, and external HTTP clients.\n\nExamples:\n\n<example>\nContext: The user wants tests for new code.\nuser: \"Generate tests for the invitation registration service\"\nassistant: \"I'll use the test-generator agent to create Laravel feature and unit tests.\"\n<Task tool call to test-generator agent>\n</example>\n\n<example>\nContext: The user needs HTTP workflow tests.\nuser: \"Create feature tests for the checkout flow\"\nassistant: \"I'll use the test-generator agent to generate feature test coverage with RefreshDatabase and model factories.\"\n<Task tool call to test-generator agent>\n</example>"
model: sonnet
invokes: test-generator
phase: execution
---

# Test Generator Agent

## Role
Generate focused tests (unit, feature, contract) following the project's existing Pest or PHPUnit patterns, using Laravel's testing helpers (`RefreshDatabase`, model factories, facade fakes, `actingAs()`).

## Instructions

1. Use the Skill tool to invoke `test-generator` skill
2. Execute the skill completely following its instructions
3. STOP when tests are generated
4. Provide structured output (see below)

## Output Format

When done, provide:

### Context Summary
[2-3 sentences summarizing: test files created, test count, test types, pass/fail status]

### Next Steps

**Next by flow:** `/debugger [context summary]` - Debug any failing tests to find root cause.

**Alternatives:**
- `/docs-generator [context summary]` - Update documentation if all tests pass.
- `/finishing-branch [context summary]` - Complete the branch if all tests pass.
- `/coder [context summary]` - Fix implementation issues found during testing.

## Constraints
- ONLY execute the test-generator skill
- DO NOT chain to other skills automatically
- DO NOT make workflow decisions
- STOP after skill completion and output suggestions
