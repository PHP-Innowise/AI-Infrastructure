#!/usr/bin/env bash
# dispatch-version: 1 (DISPATCH_VERSION in codex_dispatch.py)
# AI accelerator, Codex: an installed project's hook for a session started
# in a linked worktree of the project.
#
# Git gives a worktree only what it tracks, and an installation is often
# kept out of a project's history. Codex reads a project's .codex/ - its
# hooks.json among them - only in the folders between the session's folder
# and the nearest .git, which in a worktree is the worktree's own: started
# there, Codex ran none of the installation's hooks, so the session had no
# memory and no guards. A session in the root of an installed project puts
# one entry per project hook into your own Codex hooks ($CODEX_HOME/
# hooks.json), each running this script with the hook's name, and one named
# policy that brings the project's AGENTS.md (memory-bank/scripts/
# codex_worktrees.py in the project), and lists the root in the `projects`
# file beside it. Codex runs none of them until you trust them in its hook
# review: /hooks in its terminal, Settings > Hooks in the app.
#
# For a session in a worktree of a listed root's repository, the root's
# hook runs for the worktree: the root keeps the tooling and the memory,
# the worktree is the project, whose branch names the task (ACCELERATOR_HOME,
# _STATE_DIR and _PROJECT_DIR, as memory-bank/scripts/workspace_roots.py
# reads them). Anywhere else - the root and the folders below it, where
# Codex runs the project's own hooks, and every other project - this exits
# at once. A file named `off` beside this script turns it off for good.
hook=${1:-}
case "$hook" in ''|*/*|.*) exit 0 ;; esac
# A session a launcher lends an accelerator to - the Harness, or
# scripts/accelerator_attach.py - runs that accelerator's hooks itself and
# names it in ACCELERATOR_HOME: serving it too would run them twice.
[ -n "${ACCELERATOR_HOME:-}" ] && exit 0
# Every Codex session of the person runs this for every hook: nothing below
# starts another process until the session's folder is in a worktree of a
# listed root.
here=${BASH_SOURCE[0]}
case "$here" in */*) here=${here%/*} ;; *) here=. ;; esac
[ -e "$here/off" ] || [ ! -f "$here/projects" ] && exit 0
# Codex runs a hook in the session's folder.
cd -P -- "$PWD" 2>/dev/null || exit 0
start=$PWD
# The checkout the session's folder is in: the nearest folder with a .git.
# A linked worktree's .git is a file naming <main checkout>/.git/worktrees/
# <name>, where Git keeps a file naming the worktree back; a submodule's
# names a folder under .git/modules/ and is not one. The main checkout and
# the folders below it are Codex's own to serve.
checkout=
main=
folder=$start
while :; do
  if [ -e "$folder/.git" ]; then
    checkout=${folder:-/}
    line=
    if [ -f "$folder/.git" ] && IFS= read -r line < "$folder/.git" || [ -n "$line" ]; then
      line=${line%$'\r'}
      case "$line" in
        "gitdir: "?*)
          line=${line#gitdir: }
          # Git 2.48 writes it relative under worktree.useRelativePaths.
          case "$line" in
            /*) ;;
            */*) line=$(cd -P -- "$checkout/${line%/*}" 2>/dev/null && pwd)/${line##*/} ;;
          esac
          case "$line" in */.git/worktrees/?*) [ -f "$line/gitdir" ] && main=${line%/.git/worktrees/*} ;; esac ;;
      esac
    fi
    break
  fi
  [ -n "$folder" ] || break
  folder=${folder%/*}
done
[ -n "$main" ] || exit 0
# The longest listed root whose folder in the worktree - the worktree itself
# for the main checkout, the same folder of it for a project installed in a
# folder of a monorepo - holds the session's folder.
root=
project=
while IFS= read -r listed || [ -n "$listed" ]; do
  case "$listed" in
    "$main"|"$main"/*)
      candidate=$checkout${listed:${#main}}
      case "$start" in
        "$candidate"|"$candidate"/*) [ ${#listed} -gt ${#root} ] && root=$listed && project=$candidate ;;
      esac ;;
  esac
done < "$here/projects"
[ -n "$root" ] || exit 0
case "$hook" in
  policy) needed=AGENTS.md ;;
  *) needed=.codex/hooks/$hook ;;
esac
[ -f "$root/memory-bank/scripts/context.py" ] && [ -f "$root/$needed" ] || exit 0
# A worktree with an installation of its own between the session's folder
# and its top - Git tracks it there - runs that one's hooks.
folder=$start
while :; do
  [ -f "$folder/memory-bank/scripts/context.py" ] && exit 0
  [ ${#folder} -gt ${#checkout} ] || break
  folder=${folder%/*}
done
cd -- "$project" || exit 0
export ACCELERATOR_HOME="$root" ACCELERATOR_STATE_DIR="$root" ACCELERATOR_PROJECT_DIR="$project"
if [ "$hook" = policy ]; then
  command -v python3 > /dev/null 2>&1 || exit 0
  # Isolated and without site: the helper needs the standard library only,
  # and a PYTHONPATH of the person's cannot shadow it.
  exec python3 -I -S "$here/ai-accelerator-dispatch.py" policy "$root" "$project"
fi
exec "${BASH:-bash}" "$root/.codex/hooks/$hook"
