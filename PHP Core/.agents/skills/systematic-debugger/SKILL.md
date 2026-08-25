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

## Native PHP Debugging Toolbox

Reach for the least invasive tool that answers the question:

- **Read the error first.** PHP notices/warnings/exceptions carry file, line, and a stack trace. Enable `display_errors`/`error_reporting(E_ALL)` in dev; read `error_log` / the PSR-3 log otherwise.
- **Step debugging:** Xdebug (`xdebug.mode=debug`) with breakpoints in the IDE is the fastest way to inspect state without editing code.
- **Targeted tracing:** `var_dump()`, `var_export()`, `error_log()`, or a PSR-3 logger at the exact data-flow point identified in Phase 1 (remove before committing).
- **Assertions:** `assert()` and typed arguments/return types surface contract violations early.
- **Stack traces on demand:** `debug_print_backtrace()` or `(new \Exception())->getTraceAsString()`.
- **Database:** log the actual SQL and bound parameters; run the query manually with `EXPLAIN` when results look wrong.
- **Reproduce in a test:** encode the failing scenario as a PHPUnit/Pest test (see Phase 4) so the bug cannot silently return.
- **Bisect:** `git bisect` when a regression appeared but the cause is unclear.

Xdebug's profiler answers "why is it slow"; for deeper performance work, hand off to `performance-optimization`.

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

4. **Sweep The Root Cause**
   - The root cause is a shape, not a line. Search for that shape everywhere:
     same call, same missing guard, same wrong comparison.
   - Grep for the mechanism, not the symptom. One handler that swallows an
     expected exception and then keeps using state the failure invalidated is
     six handlers if six of them are written the same way.
   - Report every other site found, file and line, even when fixing it is out
     of scope. Fixing one instance and staying silent about the rest reads as
     "fixed" and is not.
   - This is not a "while I'm here" improvement: it is the same defect. Whether
     to fix the rest now is the requester's scope call, but they cannot make it
     without the list.
   - Record the result in the bug record's `## Same Shape Elsewhere`. An empty
     section means the search ran and found nothing, not that it was skipped.

5. **If 3+ Fixes Failed**
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

## Record The Finding

A root cause you confirmed is durable knowledge; a root cause you only
described in prose is not. This skill is declared write-capable, so record it
directly:

```bash
python3 memory-bank/scripts/context.py brain-create finding \
  --external-id <TASK>-F<N> --title "<one line>" --source <path>
```

Cite a bare path or `path#L42` — `--source` rejects `path:42`. Once the fix is
verified, close the record so the consolidation pipeline can promote it; the
two edges are separate calls:

```bash
python3 memory-bank/scripts/context.py brain-update --record-id <id> \
  --revision auto --authority verified --reason "Verified: <check>"
python3 memory-bank/scripts/context.py brain-update --record-id <id> \
  --revision auto --progress "<one-line consequence>" \
  --transition resolved --reason "Resolved"
```

`--progress` carries the record's only content. Omit it and the record is
blocked from promotion as carrying nothing beyond its own title, which is the
same dead end as never creating it.

## Next Steps

After debugging is complete and fix is verified, STOP and present these options:

**Next by flow:** [[test-generator]] `[context]` - Generate/update tests to prevent regression. See [[moc-execution]] for phase context.

**Alternatives:**
- [[documentation-generator]] `[context]` - Update documentation after the fix.
- [[code-reviewer]] `[context]` - Review the fix for quality issues.
- [[finishing-branch]] `[context]` - Complete branch if fix was the last blocker.
