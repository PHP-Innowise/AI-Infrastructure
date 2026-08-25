#!/bin/bash
# Core-changelog gate: a diff that touches shared-core files must also touch
# the root CHANGELOG.md.
#
# "Shared core" is what the root CHANGELOG.md records: the Python
# memory/context core and its tests/templates, Project Brain
# (protocol/config/schemas/scripts/tests), the per-edition tool hooks
# (canon and generated mirrors), and the repository-level tooling in
# scripts/. Infrastructure-Creator keeps its own CHANGELOG.md and is not
# gated here.
#
# Usage:
#   scripts/check_core_changelog.sh [BASE_REF]
#
# BASE_REF defaults to origin/$GITHUB_BASE_REF inside a GitHub pull_request
# job and to origin/main elsewhere (falling back to a local main when no
# origin remote exists). The diff is taken from the merge base of BASE_REF
# and HEAD against the working tree, so a local run before committing gives
# the same answer CI will give for the pushed result. When no merge base can
# be resolved (missing remote, unfetched base in a shallow clone), the check
# degrades: it explains why and exits 0 instead of failing on a question it
# cannot answer.
#
# Exit codes: 0 = pass or degraded skip; 1 = core touched without a root
# CHANGELOG.md change.
set -u

cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 0

CHANGELOG="CHANGELOG.md"

# POSIX ERE describing shared-core paths, matched against repo-relative
# names from `git diff --name-only`.
CORE_PATTERN='^(((Laravel|Symfony|PHP Core)/|Cms/wordpress/)(memory-bank/(scripts|tests|templates)/|project-brain/(PROTOCOL\.md|config/|schemas/|scripts/|tests/|templates/)|\.(claude|cursor|codex)/hooks/)|scripts/)'

# The model-facing surface: what an agent actually reads. Generated mirrors
# (.cursor, .codex) are deliberately excluded - they cannot change without
# their canon changing, and listing them would report one edit three times.
# The policy lock is excluded too: it is derived from this surface, so
# requiring a changelog entry for it would fire on its own regeneration.
SURFACE_PATTERN='^(Laravel|Symfony|PHP Core)/(AGENTS\.md|CLAUDE\.md|\.agents/skills/|\.claude/(DOD|GOLDEN-PRINCIPLES|STABILIZATION)\.md|\.claude/(agents|commands)/|\.claude/settings\.json)'

if ! git rev-parse --git-dir > /dev/null 2>&1; then
  echo "core-changelog: not a git repository; skipping." >&2
  exit 0
fi

BASE_REF="${1:-}"
if [ -z "$BASE_REF" ]; then
  if [ -n "${GITHUB_BASE_REF:-}" ]; then
    BASE_REF="origin/$GITHUB_BASE_REF"
  else
    BASE_REF="origin/main"
  fi
fi

MERGE_BASE=$(git merge-base "$BASE_REF" HEAD 2>/dev/null || true)
if [ -z "$MERGE_BASE" ] && [ "$BASE_REF" = "origin/main" ]; then
  # Local clone without an origin remote: fall back to the local main.
  if MERGE_BASE=$(git merge-base main HEAD 2>/dev/null); then
    BASE_REF="main"
  fi
fi
if [ -z "$MERGE_BASE" ]; then
  echo "core-changelog: cannot resolve a merge base with $BASE_REF (missing remote or unfetched base); skipping." >&2
  exit 0
fi

# Committed changes since the merge base plus staged and unstaged
# working-tree changes; brand-new files count once they are staged.
CHANGED=$(git diff --name-only "$MERGE_BASE" -- 2>/dev/null)
if [ -z "$CHANGED" ]; then
  echo "core-changelog: no changes against $BASE_REF ($MERGE_BASE)."
  exit 0
fi

STATUS=0

CORE_TOUCHED=$(printf '%s\n' "$CHANGED" | grep -E -- "$CORE_PATTERN" || true)
if [ -z "$CORE_TOUCHED" ]; then
  echo "core-changelog: no shared-core files touched; no root $CHANGELOG entry required."
elif printf '%s\n' "$CHANGED" | grep -qxF -- "$CHANGELOG"; then
  echo "core-changelog: shared-core files changed and the root $CHANGELOG was updated."
else
  {
    echo "core-changelog: this diff touches shared-core files but not the root $CHANGELOG:"
    printf '%s\n' "$CORE_TOUCHED" | sed 's/^/  - /'
    echo ""
    echo "Add an entry to the root $CHANGELOG (its header states the scope),"
    echo "or move the change out of the shared core."
  } >&2
  STATUS=1
fi

# Second rule, same shape, different target. The surface the model reads is
# edition content, not shared core, so it belongs in that edition's own
# changelog - and until this check existed nothing required a record of it at
# all: a skill, an agent prompt or a policy document could change with no
# entry anywhere. Infrastructure-Creator is out of scope here for the same
# reason it is out of the shared-core rule.
for EDITION in "Laravel" "Symfony" "PHP Core"; do
  SURFACE_TOUCHED=$(printf '%s\n' "$CHANGED" | grep -E -- "$SURFACE_PATTERN" | grep -F -- "$EDITION/" || true)
  [ -n "$SURFACE_TOUCHED" ] || continue
  if printf '%s\n' "$CHANGED" | grep -qxF -- "$EDITION/$CHANGELOG"; then
    echo "core-changelog: $EDITION surface changed and $EDITION/$CHANGELOG was updated."
    continue
  fi
  {
    echo "core-changelog: this diff changes the $EDITION model-facing surface but not $EDITION/$CHANGELOG:"
    printf '%s\n' "$SURFACE_TOUCHED" | sed 's/^/  - /'
    echo ""
    echo "Add an entry to $EDITION/$CHANGELOG, and regenerate the policy lock"
    echo "with: python3 scripts/policy_lock.py --write --edition \"$EDITION\""
  } >&2
  STATUS=1
done

exit "$STATUS"
