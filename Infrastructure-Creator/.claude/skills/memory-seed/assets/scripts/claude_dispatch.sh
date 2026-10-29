#!/usr/bin/env bash
# dispatch-version: 4 (DISPATCH_VERSION in claude_dispatch.py)
# AI accelerator, Claude Code: an installed project's hook for a session
# started in a folder below the project root, or in a worktree of it.
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
#
# A linked worktree of a listed root's repository - Claude Code's own under
# .claude/worktrees/, or any other - has none of an installation Git does
# not track. A session started there gets the root's hooks too, run for the
# worktree: the root keeps the tooling and the memory, the worktree is the
# project, whose branch names the task (ACCELERATOR_HOME, _STATE_DIR and
# _PROJECT_DIR, as memory-bank/scripts/workspace_roots.py reads them).
hook=${1:-}
case "$hook" in ''|*/*|.*) exit 0 ;; esac
# A session a launcher lends an accelerator to - the Harness, or
# scripts/accelerator_attach.py - runs that accelerator's hooks through
# --settings in any folder, a worktree of an installed project among them,
# and names it in ACCELERATOR_HOME: serving it too would run them twice.
[ -n "${ACCELERATOR_HOME:-}" ] && exit 0
# Every session of the person runs this for every hook: nothing below starts
# another process until the session's folder is below a listed root or in a
# worktree of one.
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
# The checkout the session's folder is in: the nearest folder with a .git.
# A linked worktree's .git is a file naming <main checkout>/.git/worktrees/
# <name>; a submodule's names a folder under .git/modules/ and is not one.
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
            /*|[A-Za-z]:[\\/]*) ;;
            */*) line=$(cd -P -- "$checkout/${line%/*}" 2>/dev/null && pwd)/${line##*/} ;;
          esac
          shell_path "$line"
          # Git keeps a file naming the worktree back in its folder there.
          case "$path" in */.git/worktrees/?*) [ -f "$path/gitdir" ] && main=${path%/.git/worktrees/*} ;; esac ;;
      esac
    fi
    break
  fi
  [ -n "$folder" ] || break
  folder=${folder%/*}
done
# Two ways to find the root: the longest listed root the session's folder
# is below, and in a worktree the longest listed root whose folder there -
# the worktree itself for the main checkout - holds the session's folder.
below=
below_native=
borrowed=
borrowed_native=
borrowed_project=
exact=
exact_native=
while IFS= read -r listed || [ -n "$listed" ]; do
  listed=${listed%$'\r'}
  shell_path "$listed"
  if [ -n "$main" ]; then
    case "$path" in
      "$main"|"$main"/*)
        candidate=$checkout${path:${#main}}
        case "$start" in
          "$candidate"|"$candidate"/*)
            [ ${#path} -gt ${#borrowed} ] && borrowed=$path && borrowed_native=$listed && borrowed_project=$candidate ;;
        esac ;;
    esac
  fi
  case "$start" in "$path"/*) [ ${#path} -gt ${#below} ] && below=$path && below_native=$listed ;; esac
  # A pattern, as above: on Windows names ignore case (nocasematch), and the
  # list's C:\ reads /C/ where the session's folder reads /c/.
  case "$start" in "$path") exact=$path exact_native=$listed ;; esac
done < "$here/projects"
# A root in the session's own checkout, or in one around it - a monorepo it
# is installed in - comes first. A folder of another checkout inside the
# project - a Git repository kept there, or a worktree of another
# repository - is that checkout's, whose branch is not the project's; a
# worktree of the project's own repository borrows the root's installation.
root=
native=
project=
if [ -n "$below" ]; then
  case "$checkout" in "$below"/*) ;; *) root=$below native=$below_native ;; esac
fi
if [ -z "$root" ] && [ -n "$borrowed" ]; then
  root=$borrowed native=$borrowed_native project=$borrowed_project
fi
# On Windows python3 is often missing, or the Store's stand-in: the first
# that runs counts.
pick_python() {
  python=()
  if [ -z "$windows" ] && command -v python3 > /dev/null 2>&1; then
    python=(python3)
    return 0
  fi
  for name in python py python3; do
    candidate=("$name")
    [ "$name" = py ] && candidate+=(-3)
    if "${candidate[@]}" -I -S -c 'import sys; sys.exit(sys.version_info < (3, 9))' > /dev/null 2>&1; then
      python=("${candidate[@]}")
      return 0
    fi
  done
  return 1
}
if [ -z "$root" ]; then
  # A session started in a listed root runs the root's own hooks, also once
  # it has entered a worktree of the root (Claude Code's EnterWorktree): they
  # serve that worktree, and the save step they ask for leaves it, which
  # Claude Code asks about there. The memory CLI alone gets the allow it has
  # in the root; the guard that ran is the root's own.
  [ -n "$exact" ] && [ "$hook" = bash-validator.sh ] || exit 0
  # Every Bash call of a session in the root passes here: read without a process.
  payload=
  IFS= read -r -d '' payload || :
  case "$payload" in *context.py*) ;; *) exit 0 ;; esac
  [[ $payload =~ \"cwd\"[[:space:]]*:[[:space:]]*\"([^\"]*)\" ]] || exit 0
  [ "${BASH_REMATCH[1]}" != "$start" ] && [ -f "$exact/memory-bank/scripts/context.py" ] && pick_python || exit 0
  printf '%s' "$payload" | "${python[@]}" -I -S "$here/ai-accelerator-dispatch.py" entered "$exact_native" "$start"
  exit $?
fi
case "$hook" in
  policy-*) needed=AGENTS.md ;;
  permissions|environment) needed=.claude/settings.json ;;
  *) needed=.claude/hooks/$hook ;;
esac
[ -f "$root/memory-bank/scripts/context.py" ] && [ -f "$root/$needed" ] || exit 0
# The nearest installed project is the session's: one installed in a folder
# between it and the root, listed or not, is not the root's to serve - nor
# a worktree with an installation of its own, whose hooks Claude Code runs.
# The folder only shortens, so the walk ends whatever the case of its names.
folder=$start
if [ -n "$project" ]; then
  while :; do
    [ -f "$folder/memory-bank/scripts/context.py" ] && exit 0
    [ ${#folder} -gt ${#project} ] || break
    folder=${folder%/*}
  done
else
  while [ ${#folder} -gt ${#root} ]; do
    [ -f "$folder/memory-bank/scripts/context.py" ] && exit 0
    folder=${folder%/*}
  done
fi
if [ -n "$project" ]; then
  cd -- "$project" || exit 0
  worktree=$project
  if [ -n "$windows" ]; then
    case "$worktree" in /[A-Za-z]|/[A-Za-z]/*) letter=${worktree:1:1}; worktree="${letter^^}:${worktree:2}" ;; esac
  fi
  export ACCELERATOR_HOME="$native" ACCELERATOR_STATE_DIR="$native" ACCELERATOR_PROJECT_DIR="$worktree"
  export CLAUDE_PROJECT_DIR="$worktree"
else
  cd -- "$root" || exit 0
  export CLAUDE_PROJECT_DIR="$native"
fi
if [ "$hook" = bash-validator.sh ]; then
  payload=$(cat)
  # The guard runs as in the root. The helper below adds the allow the
  # project's rules give the memory CLI there - the save step's command -
  # so it starts only for a command that names the CLI: its Python start
  # cost every other Bash call five times the guard's own.
  case "$payload" in
    *context.py*) ;;
    *) printf '%s' "$payload" | "${BASH:-bash}" "$root/.claude/hooks/$hook"; exit $? ;;
  esac
fi
case "$hook" in
  bash-validator.sh|permissions|environment|policy-*)
    # The guard's verdict plus what the project's permission rules allow,
    # the decisions those rules make, and the policy its .claude/CLAUDE.md
    # imports: what a session started in the root has.
    if pick_python; then
      # Isolated and without site: the helper needs the standard library
      # only, and a PYTHONPATH of the person's cannot shadow it.
      if [ "$hook" = bash-validator.sh ]; then
        printf '%s' "$payload" | "${python[@]}" -I -S "$here/ai-accelerator-dispatch.py" "$hook" "$native" "$start" ${project:+"$worktree"}
        exit $?
      fi
      exec "${python[@]}" -I -S "$here/ai-accelerator-dispatch.py" "$hook" "$native" "$start" ${project:+"$worktree"}
    fi ;;
esac
case "$hook" in permissions|environment|policy-*) exit 0 ;; esac
if [ "$hook" = bash-validator.sh ]; then
  printf '%s' "$payload" | "${BASH:-bash}" "$root/.claude/hooks/$hook"
  exit $?
fi
exec "${BASH:-bash}" "$root/.claude/hooks/$hook"
