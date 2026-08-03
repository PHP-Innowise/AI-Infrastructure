#!/bin/bash
# Local Context Hook (Infrastructure-Creator)
# Prints a short orientation banner at session start.
# Hook type: SessionStart / sessionStart
# Exit codes: 0 = always continue (context only, never blocks)

set -euo pipefail

cat <<'BANNER'
[Infrastructure-Creator] You are running a PHP accelerator GENERATOR, not an accelerator.
- Every skill requires an explicit TARGET PROJECT PATH (e.g. "infra-scan ../my-php-app").
- Phase 1 (infra-scan) is READ-ONLY on the target. Only Phase 2 (infra-generate) writes into it.
- Generate ONLY the AI-tool edition(s) selected in clarifying-interview.
- Never read/print/copy the target's .env or secrets. Cite file evidence for every finding.
BANNER

# Installation version: VERSION at the generator root names the latest
# released section of Infrastructure-Creator/CHANGELOG.md.
edition_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ -f "$edition_dir/VERSION" ]; then
  echo "[Infrastructure-Creator] Version: $(tr -d '[:space:]' < "$edition_dir/VERSION" 2>/dev/null || true)"
fi

# Loop detection is session-scoped; discard counters from earlier sessions.
# Counters are namespaced by a stable hash of the repo root; only reset ours.
repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
repo_key="$(printf '%s' "$repo_root" | cksum | cut -d' ' -f1)"
find "${TMPDIR:-/tmp}/infra-creator-loopdetect-$repo_key" -type f -delete 2>/dev/null || true

# Surface the current run's task counter if present, to help resume work.
counter_file="tasks/.task-counter"
if [ -f "$counter_file" ]; then
  echo "[Infrastructure-Creator] Next task id: $(cat "$counter_file" 2>/dev/null || echo '?')"
fi

exit 0
