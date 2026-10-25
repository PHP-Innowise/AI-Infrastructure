#!/usr/bin/env bash
# AI accelerator, Claude Code: an installed project's hook for a session
# started in a folder below the project root.
#
# Claude Code reads a project's settings - its hooks and permission rules -
# only from the folder a session starts in: started in src/ of an installed
# project, it ran none of the project's hooks, so the session had no memory
# and no guards. A session in the root of an installed project puts one
# entry per project hook into your own Claude Code settings, each running
# this script with the hook's name (memory-bank/scripts/claude_subfolders.py
# in the project), and lists the root in the `projects` file beside it.
# A session started in the root runs the project's own hooks and this exits
# at once, as it does for a folder below no listed root and for a folder of
# another Git checkout inside one. The entries named policy-1, policy-2, ...
# bring the project's AGENTS.md in parts. A file named `off` beside this
# script turns it off for good.
hook=${1:-}
case "$hook" in ''|*/*|.*) exit 0 ;; esac
# Every session of the person runs this for every hook: nothing below starts
# another process until the session's folder is below a listed root.
case "${BASH_SOURCE[0]}" in */*) here=${BASH_SOURCE[0]%/*} ;; *) here=. ;; esac
[ -e "$here/off" ] || [ ! -f "$here/projects" ] && exit 0
cd -P -- "${CLAUDE_PROJECT_DIR:-$PWD}" 2>/dev/null || exit 0
start=$PWD
root=
while IFS= read -r listed || [ -n "$listed" ]; do
  case "$start" in "$listed"/*) [ ${#listed} -gt ${#root} ] && root=$listed ;; esac
done < "$here/projects"
[ -n "$root" ] || exit 0
case "$hook" in
  policy-*) needed=AGENTS.md ;;
  *) needed=.claude/hooks/$hook ;;
esac
[ -f "$root/memory-bank/scripts/context.py" ] && [ -f "$root/$needed" ] || exit 0
# The nearest installed project is the session's: one installed in a folder
# between it and the root, listed or not, is not the root's to serve.
folder=$start
while [ "$folder" != "$root" ]; do
  [ -f "$folder/memory-bank/scripts/context.py" ] && exit 0
  folder=${folder%/*}
done
# A folder of another checkout - a Git worktree or a repository kept inside
# the project - is that checkout's, whose branch is not the project's. A
# checkout around the project, a monorepo it is installed in, is not.
top=$(git -C "$start" rev-parse --show-toplevel 2>/dev/null) && case "$top" in "$root"/*) exit 0 ;; esac
cd -- "$root" || exit 0
export CLAUDE_PROJECT_DIR="$root"
case "$hook" in
  bash-validator.sh|policy-*)
    # The guard's verdict plus what the project's permission rules allow,
    # and the policy its .claude/CLAUDE.md imports: what a session started
    # in the root has.
    if command -v python3 > /dev/null 2>&1; then
      exec python3 "$here/ai-accelerator-dispatch.py" "$hook" "$root" "$start"
    fi ;;
esac
case "$hook" in policy-*) exit 0 ;; esac
exec bash "$root/.claude/hooks/$hook"
