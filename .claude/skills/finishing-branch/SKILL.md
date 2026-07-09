---
name: finishing-branch
description: Use when implementation is complete, all tests pass, and you need to decide how to integrate the work - guides completion of development work by presenting structured options for merge, PR, or cleanup
phase: execution
flow-alternatives: [documentation-generator]
related: [release, code-reviewer]
---

# Finishing Branch

## Overview

Guide completion of development work by presenting clear options and handling chosen workflow.

## Generated File Naming Convention (MANDATORY)

**ANY file created by this skill MUST be prefixed with `finishing-branch-`:**
- ✅ `finishing-branch-summary.md`, `finishing-branch-pr-description.md`
- ❌ `BRANCH_SUMMARY.md`, `PR_DESCRIPTION.md`

This applies to ALL generated files — summaries, reports.

**Core principle:** Verify tests → Present options → Execute choice → Clean up.

**Prerequisite:** Consider running `/docs-generator` and `/release` first to update documentation and create a release before completing the branch.

**Announce at start:** "I'm using the finishing-branch skill to complete this work."

## The Process

### Step 1: Verify Tests

**Before presenting options, verify tests pass:**

```bash
composer validate --strict
composer test        # or vendor/bin/phpunit / vendor/bin/pest
composer lint        # or vendor/bin/php-cs-fixer fix --dry-run --diff
composer analyse     # or vendor/bin/phpstan analyse / vendor/bin/psalm
```

**If tests fail:** Stop. Don't proceed. Fix issues first.

**If tests pass:** Continue to Step 2.

### Step 2: Determine Base Branch

```bash
git merge-base HEAD main 2>/dev/null || git merge-base HEAD master 2>/dev/null
```

Or ask: "This branch split from main - is that correct?" **Use AskUserQuestion tool.**

### Step 3: Present Options

Present exactly these 4 options. **Use AskUserQuestion tool.**

```
Implementation complete. What would you like to do?

1. Merge back to <base-branch> locally
2. Push and create a Pull Request
3. Keep the branch as-is (I'll handle it later)
4. Discard this work

Which option?
```

### Step 4: Execute Choice

#### Option 1: Merge Locally

```bash
git checkout <base-branch>
git pull
git merge <feature-branch>
composer test  # Verify on merged result
git branch -d <feature-branch>
```

Then: Cleanup worktree (Step 5)

#### Option 2: Push and Create PR

```bash
git push -u origin <feature-branch>

gh pr create --title "<title>" --body "$(cat <<'EOF'
## Summary
<2-3 bullets of what changed>

## Test Plan
- [ ] <verification steps>

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

Then: Report PR URL

#### Option 3: Keep As-Is

Report: "Keeping branch <name>. Worktree preserved at <path>."

**Don't cleanup worktree.**

#### Option 4: Discard

**Confirm first. Use AskUserQuestion tool.**
```
This will permanently delete:
- Branch <name>
- All commits
- Worktree at <path>

Type 'discard' to confirm.
```

Wait for exact confirmation. If confirmed:
```bash
git checkout <base-branch>
git branch -D <feature-branch>
```

### Step 5: Cleanup Worktree

**For Options 1, 2, 4:**

```bash
git worktree list | grep $(git branch --show-current)
# If in worktree:
git worktree remove <worktree-path>
```

**For Option 3:** Keep worktree.

## Quick Reference

| Option | Merge | Push | Keep Worktree | Cleanup Branch |
|--------|-------|------|---------------|----------------|
| 1. Merge locally | ✓ | - | - | ✓ |
| 2. Create PR | - | ✓ | ✓ | - |
| 3. Keep as-is | - | - | ✓ | - |
| 4. Discard | - | - | - | ✓ (force) |

## Red Flags

**Never:**
- Proceed with failing tests
- Merge without verifying tests on result
- Delete work without confirmation
- Force-push without explicit request

---

## Next Steps

After branch is finished (merged, PR created, or kept), STOP and present these options:

**Next by flow:** [[/verify]] `[context]` - Verify implementation meets requirements. See [[moc-execution]] for phase context.

**Alternatives:**
- [[/docs-generator]] `[context]` - Update documentation for the changes.
