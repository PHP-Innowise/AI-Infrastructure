---
name: systematic-debugger
description: Use when encountering any bug, test failure, or unexpected behavior, before proposing fixes. Requires root cause investigation before any fixes. Triggers on "debug", "error", "fix bug", "test failure", "investigate", "not working".
phase: execution
flow-next: test-generator
flow-alternatives: [coder, coder-frontend, browser-verify]
related: [test-generator, coder, browser-verify]
---

# Systematic Debugger

## Overview

Find root cause before attempting fixes. Random fixes waste time and create new bugs.

**Core principle:** ALWAYS find root cause before attempting fixes. Symptom fixes are failure.

## Generated File Naming Convention (MANDATORY)

**ANY file created by this skill MUST be prefixed with `systematic-debugger-`:**
- ✅ `systematic-debugger-investigation.md`, `systematic-debugger-root-cause.md`
- ❌ `DEBUG_NOTES.md`, `INVESTIGATION.md`

This applies to ALL generated files — investigation reports, root cause analyses, debug logs.

## The Iron Law

```
NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST
```

If you haven't completed Phase 1, you cannot propose fixes.

## When to Use

Use for ANY technical issue:
- Test failures
- Bugs in production
- Unexpected behavior
- Performance problems
- Build failures
- Integration issues

## The Four Phases

### Phase 1: Root Cause Investigation

**BEFORE attempting ANY fix:**

1. **Read Error Messages Carefully**
   - Don't skip past errors
   - Read stack traces completely
   - Note line numbers, file paths, error codes

2. **Reproduce Consistently**
   - Can you trigger it reliably?
   - What are the exact steps?
   - If not reproducible → gather more data

3. **Check Recent Changes**
   - What changed that could cause this?
   - Git diff, recent commits
   - New dependencies, config changes

4. **Trace Data Flow**
   - Where does bad value originate?
   - What called this with bad value?
   - Keep tracing up until you find the source

### Phase 2: Pattern Analysis

1. **Find Working Examples**
   - Locate similar working code in same codebase
   - What works that's similar to what's broken?

2. **Compare Against References**
   - Read reference implementation COMPLETELY
   - Don't skim - read every line

3. **Identify Differences**
   - What's different between working and broken?
   - List every difference, however small

### Phase 3: Hypothesis and Testing

1. **Form Single Hypothesis**
   - State clearly: "I think X is the root cause because Y"
   - Be specific, not vague

2. **Test Minimally**
   - Make the SMALLEST possible change
   - One variable at a time
   - Don't fix multiple things at once

3. **Verify Before Continuing**
   - Did it work? Yes → Phase 4
   - Didn't work? Form NEW hypothesis
   - DON'T add more fixes on top

### Phase 4: Implementation

1. **Create Failing Test Case**
   - Simplest possible reproduction
   - MUST have before fixing

2. **Implement Single Fix**
   - Address the root cause identified
   - ONE change at a time
   - No "while I'm here" improvements

3. **Verify Fix**
   - Test passes now?
   - No other tests broken?
   - Issue actually resolved?

4. **If 3+ Fixes Failed**
   - STOP and question the architecture
   - 3+ failures = architectural problem
   - Discuss with human partner before continuing

## Red Flags - STOP

If you catch yourself thinking:
- "Quick fix for now, investigate later"
- "Just try changing X and see if it works"
- "Add multiple changes, run tests"
- "Skip the test, I'll manually verify"
- "I don't fully understand but this might work"

**ALL of these mean: STOP. Return to Phase 1.**

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "Issue is simple, don't need process" | Simple issues have root causes too |
| "Emergency, no time for process" | Systematic is FASTER than thrashing |
| "Just try this first, then investigate" | First fix sets the pattern. Do it right. |
| "I'll write test after confirming fix works" | Untested fixes don't stick |
| "Multiple fixes at once saves time" | Can't isolate what worked |

## Quick Reference

| Phase | Key Activities | Success Criteria |
|-------|---------------|------------------|
| 1. Root Cause | Read errors, reproduce, trace | Understand WHAT and WHY |
| 2. Pattern | Find working examples, compare | Identify differences |
| 3. Hypothesis | Form theory, test minimally | Confirmed or new hypothesis |
| 4. Implementation | Create test, fix, verify | Bug resolved, tests pass |

---

## Next Steps

After debugging is complete and fix is verified, STOP and present these options:

**Next by flow:** [[/test-generator]] `[context]` - Generate/update tests to prevent regression. See [[moc-execution]] for phase context.

**Alternatives:**
- [[/docs-generator]] `[context]` - Update documentation after the fix.
- [[/code-reviewer]] `[context]` - Review the fix for quality issues.
- [[/finishing-branch]] `[context]` - Complete branch if fix was the last blocker.
