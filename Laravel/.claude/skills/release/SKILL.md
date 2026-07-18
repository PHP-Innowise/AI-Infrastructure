---
name: release
description: Prepare Laravel project releases with changelog generation, version tags, and optional GitHub publishing. Use when creating a release, generating changelogs, tagging versions, or publishing release notes.
phase: finalization
flow-next: finishing-branch
flow-alternatives: []
related: [documentation-generator]
---

# Release

## Overview

Prepare releases with automated changelog generation from git commits. Analyze commits since the last tag, categorize changes using conventional commits, update `CHANGELOG.md`, create a version tag, and optionally publish a GitHub release if the project uses GitHub.

## Announce at Start

Start by announcing: "Creating release with changelog generation..."

## Generated File Naming Convention (MANDATORY)

**ANY documentation or temporary file created by this skill MUST be prefixed with `release-`:**
- ✅ `release-commit-analysis.md`, `release-summary.md`
- ❌ `COMMIT_ANALYSIS.md`, `SUMMARY.md`

Standard project files (`CHANGELOG.md`, git tags) are exempt from this rule.

## Release Workflow

```
1. DISCOVER → 2. ANALYZE → 3. VERSION → 4. CHANGELOG → 5. PUBLISH
```

### Step 1: Discover Last Release

Find the last release or tag to determine the starting point for changes:

```bash
# Find last tag
git describe --tags --abbrev=0

# List recent GitHub releases if GitHub CLI is configured
gh release list --limit 5
```

**If no tags exist:** Use the initial commit as the starting point:
```bash
git rev-list --max-parents=0 HEAD
```

### Step 2: Analyze Commits

Collect and analyze all commits since the last release:

```bash
# Get commit list
git log <last-tag>..HEAD --pretty=format:"%h %s" > /tmp/commits.txt

# Get file statistics
git diff --stat <last-tag>..HEAD

# Get changed files
git diff --name-only <last-tag>..HEAD
```

**Categorize commits** using the bundled script:
```bash
python3 .claude/skills/release/scripts/categorize_commits.py /tmp/commits.txt
```

The script categorizes commits into:
- **Added** - New features (feat:, feature:)
- **Changed** - Modifications (refactor:, perf:, update:)
- **Fixed** - Bug fixes (fix:, bugfix:)
- **Deprecated** - Deprecations (deprecate:)
- **Removed** - Deletions (remove:, delete:)
- **Security** - Security fixes (security:, sec:)

See `references/changelog-format.md` for detailed format guidance.

### Step 3: Determine Version

**Parse the last tag** to suggest the next version:
```bash
# Last tag format: v1.2.3
LAST_TAG=$(git describe --tags --abbrev=0)
# Parse to get MAJOR.MINOR.PATCH
```

**Ask user for version type** using AskUserQuestion:

```
What type of release is this?

1. Patch (x.x.X) - Bug fixes, minor changes
2. Minor (x.X.0) - New features, backwards compatible
3. Major (X.0.0) - Breaking changes

Current version: vX.Y.Z
Suggested next versions:
- Patch: vX.Y.(Z+1)
- Minor: vX.(Y+1).0
- Major: v(X+1).0.0
```

**Calculate the new version** based on user selection.

### Step 4: Update CHANGELOG.md

Generate the changelog entry using Keep a Changelog format:

```markdown
## [VERSION] - YYYY-MM-DD

### Added
- Description (commit-hash)

### Changed
- Description (commit-hash)

### Fixed
- Description (commit-hash)
```

**Only include sections with entries.** Skip empty categories.

**If CHANGELOG.md exists:**
- Read the current file
- Insert the new entry after the header, before previous entries
- Preserve the existing format

**If CHANGELOG.md does not exist:**
- Create it with the standard header
- Add the new entry

**Show the changelog entry to the user** and ask for confirmation using AskUserQuestion:
```
Here's the generated changelog entry:

[show formatted entry]

Update CHANGELOG.md with this entry? (yes/no)
```

**Write the file** only after confirmation.

### Step 5: Publish Release

**Verify the release branch is green before tagging.** Run the project's Laravel verification commands and confirm they pass:

```bash
php artisan test        # or vendor/bin/pest / vendor/bin/phpunit
vendor/bin/pint --test
vendor/bin/phpstan analyse
```

If any check fails, stop and fix it before continuing — never tag a release on top of a failing test suite or static analysis run.

**Also confirm deployment readiness:**
- If this release includes a Laravel major-version bump, verify no queued jobs are in-flight with a payload shape incompatible with the new version (drain the queue, or keep a compatible worker running through the cutover).
- Verify the deploy process invalidates and regenerates config/route/view caches (`config:cache`, `route:cache`, `view:cache`) rather than leaving them stale from before the deploy.

**Commit, tag, and optionally publish** the release. First determine the active release branch:

```bash
git branch --show-current
git remote -v
```

Then ask the user to confirm the target branch and whether to publish a GitHub release.

```bash
# Stage CHANGELOG.md
git add CHANGELOG.md

# Commit the changelog
git commit -m "chore: release vX.X.X"

# Create annotated tag
git tag -a vX.X.X -m "Release vX.X.X"

# Push the confirmed release branch and tags
git push origin <release-branch> --tags

# Create GitHub release if the project uses GitHub releases
gh release create vX.X.X \
  --title "vX.X.X" \
  --notes "[changelog entry text]"
```

**Ask for confirmation** before pushing using AskUserQuestion:
```
Ready to publish release vX.X.X?

This will:
1. Commit CHANGELOG.md changes
2. Create git tag vX.X.X
3. Push to origin/<release-branch> with tags
4. Create GitHub release if selected

Proceed? (yes/no)
```

**Execute the commands** only after confirmation.

### Step 6: Show Result

Display the release summary:

```
✅ Release vX.X.X published successfully!

📦 GitHub Release: https://github.com/user/repo/releases/tag/vX.X.X

📝 Changes:
- X features added
- Y bugs fixed
- Z files changed
```

## Red Flags / Never

**NEVER:**
- Skip user confirmation before writing CHANGELOG.md
- Skip user confirmation before pushing tags or creating releases
- Include empty categories in the changelog
- Use past tense in changelog entries (use imperative mood)
- Forget to push tags along with commits
- Create releases without updating CHANGELOG.md first
- Include "chore:", "docs:", or "build:" commits in user-facing changelog sections

**ALWAYS:**
- Use semantic versioning (MAJOR.MINOR.PATCH)
- Follow Keep a Changelog format
- Ask for version type (patch/minor/major)
- Show generated changelog before writing
- Confirm before publishing
- Include commit hashes in changelog entries
- Use imperative mood in descriptions

## Handling Edge Cases

**No previous tags:**
- Use initial commit as starting point
- First version should be v0.1.0 or v1.0.0
- Ask user which to use

**Uncommitted changes:**
- Warn user about uncommitted changes
- Ask to commit or stash before proceeding

**Existing tag:**
- Check if tag already exists
- Prevent overwriting existing tags
- Suggest incrementing version

**Failed push:**
- Show error message
- Suggest checking remote access
- Keep local changes intact

## Next Steps

After release creation is complete:

**Next by flow:** [[/finishing-branch]] `[release summary]` - Complete the branch and prepare for merge/PR. See [[moc-finalization]] for phase context.

**Alternatives:**
- No further action needed - Release is complete and published.
