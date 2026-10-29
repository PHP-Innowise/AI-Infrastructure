"""AI accelerator, Codex: the project's AGENTS.md for a session started in a
linked worktree of an installed project (see ai-accelerator-dispatch.sh
beside this file).

Codex reads AGENTS.md in the folders between the session's folder and the
nearest .git, which in a worktree is the worktree's own, so a worktree
without the installation has none of the project's policy. The entry named
policy brings the main checkout's, whole, as the context of the session's
start: Codex keeps a hook's context up to the entry's additionalContextLimit
(POLICY_LIMIT, counted in its tokens) and cuts a longer one, keeping the rest
in a file. Nothing comes when the worktree has an AGENTS.md of its own, which
Codex reads itself, nor when a session resumes, whose history holds the
policy of its start: Codex replays it there and runs the start hooks again.

Usage: ai-accelerator-dispatch.py policy ROOT PROJECT, the hook's input on
stdin.
"""
import json
import sys
from pathlib import Path

# One copy serves every project of the person: a session in the root of a
# project with an older release must not put an older one back. Raise it,
# with the comment in codex_dispatch.sh, whenever either file changes.
DISPATCH_VERSION = 1
# The policy entry's additionalContextLimit: the AGENTS.md chain the
# editions let Codex read in the root (project_doc_max_bytes, 131,072 bytes),
# at about four bytes a token.
POLICY_LIMIT = 32768
# The session starts that have no policy in their history yet: a new one, a
# cleared one and a compacted one, whose history is a summary.
FRESH = ("startup", "clear", "compact")


def policy(root: str, project: str, payload: object) -> str:
    """The project's AGENTS.md with a line saying what it is, or nothing."""
    source = payload.get("source", "startup") if isinstance(payload, dict) else "startup"
    if source not in FRESH:
        return ""
    path = Path(root) / "AGENTS.md"
    if not path.is_file() or (Path(project) / "AGENTS.md").is_file():
        return ""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        return ""
    return f"Contents of {path} (the project's policy):\n\n{text}"


def main(argv: list) -> int:
    if len(argv) != 4 or argv[1] != "policy":
        return 0
    try:
        payload = json.loads(sys.stdin.buffer.read(1024 * 1024) or b"{}")
    except ValueError:
        payload = {}
    text = policy(argv[2], argv[3], payload)
    if text:
        # ASCII JSON: whatever encoding the hook's locale gives stdout.
        sys.stdout.write(json.dumps({"hookSpecificOutput": {"hookEventName": "SessionStart",
                                                            "additionalContext": text}}))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except Exception:  # a session's start is never failed by this
        sys.exit(0)
