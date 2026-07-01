---
name: review-pr
description: Review a GitHub pull request using gh CLI, analyze the diff for bugs, security issues, and code quality problems, then let the user choose next action - either run the systematic debugger to fix issues locally or post review comments directly to the PR on GitHub. Use this skill whenever the user wants to review a PR, check a pull request, audit PR changes, or analyze a GitHub PR. Triggers on "review PR", "check PR", "review pull request", "PR review", "look at this PR", "audit PR", gh pr references, or GitHub PR URLs.
phase: execution
flow-next: debugger
flow-alternatives: [coder, code-reviewer, finishing-branch]
related: [code-reviewer, debugger, coder]
---

# PR Reviewer

Review GitHub pull requests using the `gh` CLI, find issues, and take action -- either fix them locally or post comments back to the PR.

## Prerequisites

This skill requires the [GitHub CLI](https://cli.github.com/) (`gh`) to be installed and authenticated. If `gh` is not available, tell the user to install it and run `gh auth login`.

## Generated File Naming Convention (MANDATORY)

**ANY file created by this skill MUST be prefixed with `review-pr-`:**
- OK: `review-pr-findings.md`, `review-pr-analysis.md`
- Not OK: `PR_REVIEW.md`, `FINDINGS.md`

## Workflow

### Step 1: Identify the PR

The user may provide:
- A PR number (e.g., `#42` or `42`)
- A PR URL (e.g., `https://github.com/owner/repo/pull/42`)
- Nothing -- in that case, list recent open PRs with `gh pr list` and ask which one to review

Extract the PR number. If a URL is provided, parse the number from it.

### Step 2: Gather PR Context

Run these commands to collect information about the PR:

```bash
# PR metadata (title, body, author, base branch, state, labels)
gh pr view <number> --json title,body,author,baseRefName,headRefName,state,labels,additions,deletions,changedFiles

# The full diff
gh pr diff <number>

# Existing review comments (to avoid duplicating feedback)
gh pr view <number> --json reviews,comments
```

Read the diff carefully. If the diff is very large, focus on the most impactful files first -- look at the changed file list and prioritize:
1. Business logic over config/lockfiles
2. New files over minor edits
3. Files with security-sensitive operations (auth, DB queries, input handling)

### Step 3: Analyze the Changes

Review the diff against these categories (same rigor as the code-reviewer skill):

**Critical (must fix before merge):**
- Security vulnerabilities (injection, auth bypass, secrets in code)
- Data loss or corruption risks
- Crashes or unhandled errors in critical paths

**Major (should fix):**
- Bugs and logic errors
- Missing error handling
- Performance problems (N+1 queries, unbounded loops)
- Missing validation on external input
- Breaking changes without migration path

**Minor (consider fixing):**
- Code style inconsistencies
- Naming improvements
- Missing tests for new logic
- Documentation gaps

**Positive notes:**
- Well-structured code
- Good test coverage
- Clean abstractions

### Step 4: Present Findings

Present the review in this format:

```markdown
## PR Review: #<number> - <title>

**Author:** <author> | **Base:** <base> <- <head> | **Changes:** +<additions> -<deletions> across <files> files

### Summary
<1-2 sentence overall assessment>

### Critical Issues
- [ ] <issue> -- `file:line` -- <explanation>

### Major Issues
- [ ] <issue> -- `file:line` -- <explanation>

### Minor Issues
- [ ] <issue> -- `file:line` -- <explanation>

### Positive Notes
- <what was done well>
```

### Step 5: Ask User for Next Action

After presenting findings, STOP and ask the user what they want to do next:

**Option A: Fix issues locally with the debugger**
Use `/debugger` to investigate and fix the bugs found in the review. This is the right choice when the PR is yours or you're a collaborator who can push fixes.

**Option B: Post comments to the PR on GitHub**
Submit the review findings as comments directly on the PR using `gh`. This is the right choice when you want the PR author to see your feedback on GitHub.

**Option C: Do nothing**
Just keep the review for reference without taking action.

### Executing Option A: Fix Locally

1. Check out the PR branch locally:
   ```bash
   gh pr checkout <number>
   ```
2. Hand off to [[/debugger]] with context about which issues to investigate and fix.

### Executing Option B: Post Comments to PR

Use `gh` to post the review. There are two approaches depending on what the user wants:

**Approach 1: Single review comment (summary)**
Post the full review as a single PR review:
```bash
gh pr review <number> --comment --body "$(cat <<'EOF'
## Code Review

### Summary
...

### Issues Found
...
EOF
)"
```

**Approach 2: Inline comments on specific lines**
For precise feedback on specific lines, use the GitHub API to post review comments on the diff:

```bash
# Start a review and post line-level comments
gh api repos/{owner}/{repo}/pulls/<number>/reviews \
  --method POST \
  --field body="Code review summary" \
  --field event="COMMENT" \
  --field comments='[
    {
      "path": "app/Http/Controllers/ExampleController.php",
      "line": 42,
      "body": "Bug: This condition is inverted -- should be `!==` instead of `===`"
    }
  ]'
```

Ask the user which approach they prefer. If they have specific line-level issues (Critical/Major), recommend Approach 2 for those and a summary comment for the rest.

**Review event types:**
- `COMMENT` -- neutral feedback, no approval/rejection
- `REQUEST_CHANGES` -- blocks the PR until addressed
- `APPROVE` -- approves the PR

Ask the user which event type to use. Default to `COMMENT` unless the user explicitly wants to approve or request changes.

## Tips

- If the PR has many files, the user might say "just review the backend" or "focus on the API changes" -- respect scope limitations.
- When posting inline comments, map the issue to the correct file path and line number from the diff. Use the diff hunk context to get the right line in the new version of the file (the `+` side).
- If `gh` commands fail with auth errors, suggest `gh auth status` to diagnose and `gh auth login` to fix.
- For PRs from forks, `gh pr checkout` handles the remote setup automatically.

---

## Next Steps

After PR review is complete, STOP and present these options:

**Next by flow:** [[/debugger]] `[context]` - Fix bugs found in the review.

**Alternatives:**
- [[/coder]] `[context]` - Implement fixes for issues found.
- [[/code-reviewer]] `[context]` - Do a deeper local code review.
- [[/finishing-branch]] `[context]` - Complete branch if review passes.
