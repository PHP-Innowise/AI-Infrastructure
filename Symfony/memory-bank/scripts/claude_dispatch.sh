#!/usr/bin/env bash
# dispatch-version: 2 (DISPATCH_VERSION in claude_dispatch.py)
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
# another Git checkout inside one. The entry named permissions makes the
# decisions the project's permission rules make in the root, environment
# gives the session's commands the project's env, and those named policy-1,
# policy-2, ... bring the project's AGENTS.md in parts. A file named `off`
# beside this script turns it off for good. On Windows Claude Code runs it
# with Git Bash.
hook=${1:-}
case "$hook" in ''|*/*|.*) exit 0 ;; esac
# Every session of the person runs this for every hook: nothing below starts
# another process until the session's folder is below a listed root.
here=${BASH_SOURCE[0]//\\//}
case "$here" in */*) here=${here%/*} ;; *) here=. ;; esac
[ -e "$here/off" ] || [ ! -f "$here/projects" ] && exit 0
cd -P -- "${CLAUDE_PROJECT_DIR:-$PWD}" 2>/dev/null || exit 0
start=$PWD
# Git Bash: the list holds roots as Python writes them (C:\Users\me\shop),
# the session's folder reads /c/Users/me/shop, and names ignore case.
windows=
[ "${OS:-}" = "Windows_NT" ] && windows=1 && shopt -s nocasematch
shell_path() {
  path=$1
  if [ -n "$windows" ]; then
    case "$path" in [A-Za-z]:[\\/]*) path="/${path:0:1}${path:2}" ;; esac
    path=${path//\\//}
  fi
}
root=
native=
while IFS= read -r listed || [ -n "$listed" ]; do
  listed=${listed%$'\r'}
  shell_path "$listed"
  case "$start" in "$path"/*) [ ${#path} -gt ${#root} ] && root=$path && native=$listed ;; esac
done < "$here/projects"
[ -n "$root" ] || exit 0
case "$hook" in
  policy-*) needed=AGENTS.md ;;
  permissions|environment) needed=.claude/settings.json ;;
  *) needed=.claude/hooks/$hook ;;
esac
[ -f "$root/memory-bank/scripts/context.py" ] && [ -f "$root/$needed" ] || exit 0
# The nearest installed project is the session's: one installed in a folder
# between it and the root, listed or not, is not the root's to serve. The
# folder only shortens, so the walk ends whatever the case of its names.
folder=$start
while [ ${#folder} -gt ${#root} ]; do
  [ -f "$folder/memory-bank/scripts/context.py" ] && exit 0
  folder=${folder%/*}
done
# A folder of another checkout - a Git worktree or a repository kept inside
# the project - is that checkout's, whose branch is not the project's. A
# checkout around the project, a monorepo it is installed in, is not.
if top=$(git -C "$start" rev-parse --show-toplevel 2>/dev/null); then
  shell_path "$top"
  case "$path" in "$root"/*) exit 0 ;; esac
fi
cd -- "$root" || exit 0
export CLAUDE_PROJECT_DIR="$native"
case "$hook" in
  bash-validator.sh|permissions|environment|policy-*)
    # The guard's verdict plus what the project's permission rules allow,
    # the decisions those rules make, and the policy its .claude/CLAUDE.md
    # imports: what a session started in the root has. On Windows python3
    # is often missing, or the Store's stand-in: the first that runs counts.
    python=()
    if [ -z "$windows" ] && command -v python3 > /dev/null 2>&1; then
      python=(python3)
    else
      for name in python py python3; do
        candidate=("$name")
        [ "$name" = py ] && candidate+=(-3)
        if "${candidate[@]}" -I -S -c 'import sys; sys.exit(sys.version_info < (3, 9))' > /dev/null 2>&1; then
          python=("${candidate[@]}")
          break
        fi
      done
    fi
    if [ ${#python[@]} -gt 0 ]; then
      exec "${python[@]}" "$here/ai-accelerator-dispatch.py" "$hook" "$native" "$start"
    fi ;;
esac
case "$hook" in permissions|environment|policy-*) exit 0 ;; esac
exec "${BASH:-bash}" "$root/.claude/hooks/$hook"
