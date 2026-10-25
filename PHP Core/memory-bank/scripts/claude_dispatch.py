"""AI accelerator, Claude Code: what a session started below the project root
needs beyond the output of the project's own hooks (see
ai-accelerator-dispatch.sh beside this file).

bash-validator.sh: the guard runs as it does in the project root. A command
that then runs nothing but this project's memory CLI - the save step's
command among them - gets the allow the project's permission rules give it
in the root, which such a session does not read, so it is not asked about
the save after every answer.

policy-N: part N of the project's AGENTS.md. Claude Code reaches it from
.claude/CLAUDE.md through an import from outside the session's folder, which
it asks about in a terminal and leaves out under -p. It keeps a hook's output
in the conversation only up to INLINE characters and puts a longer one in a
file, of which the model sees the first 2,000 - so the policy comes in parts,
one settings entry each, cut between sections.

Usage: ai-accelerator-dispatch.py HOOK ROOT START, the hook's input on stdin.
"""
import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

INTERPRETERS = (["python3"], ["python"], ["py", "-3"])
CLI = "memory-bank/scripts/context.py"
# Settings entries that carry AGENTS.md; a policy longer than they hold ends
# in the last one, which Claude Code then keeps in a file.
POLICY_PARTS = 4
# Claude Code 2.1: the longest hook output kept in the conversation, counted
# as JavaScript counts a string's length.
INLINE = 10_000


def plain(command: str) -> bool:
    """No double quote, expansion, substitution, redirection or second line
    outside single quotes: what shlex reads is what bash would run. The one
    escape allowed is a quote's own, the save step's '\\'' inside its JSON."""
    quoted, index = False, 0
    while index < len(command):
        character = command[index]
        if character == "'":
            quoted = not quoted
        elif not quoted and character == "\\" and command[index + 1:index + 2] == "'":
            index += 1
        elif not quoted and character in "$`\n\r\\<>\"":
            return False
        index += 1
    return not quoted


def memory_command(command: str, root: str) -> bool:
    """Whether `command` runs nothing but this project's memory CLI, after an
    optional `cd ROOT &&` and an echo or printf piped into it."""
    if not isinstance(command, str) or len(command) > 64 * 1024 or not plain(command):
        return False
    try:
        lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
        lexer.whitespace_split = True
        words = list(lexer)
    except ValueError:
        return False
    relative = False
    if words[:1] == ["cd"]:
        if len(words) < 3 or words[1] != root or words[2] != "&&":
            return False
        words, relative = words[3:], True
    if words[:1] in (["echo"], ["printf"]):
        pipe = words.index("|") if "|" in words else -1
        if pipe < 2 or any(word in ("&&", "||", ";", "&", "|&") for word in words[1:pipe]):
            return False
        words = words[pipe + 1:]
    for interpreter in INTERPRETERS:
        if words[:len(interpreter)] == interpreter:
            words = words[len(interpreter):]
            break
    else:
        return False
    script = words[0] if words else ""
    if script != os.path.join(root, CLI) and not (relative and script == CLI):
        return False
    # The rest is the CLI's own arguments: nothing that starts another command.
    return not any(set(word) <= set("();<>|&") for word in words[1:])


def main_checkout(checkout: Path) -> Path:
    """The repository's main working tree when `checkout` is a linked one
    (its .git a file naming a folder under the main .git/worktrees that
    names it back), else `checkout` - as Claude Code 2.1 works it out."""
    try:
        line = (checkout / ".git").read_text(encoding="utf-8").strip()
        if not line.startswith("gitdir:"):
            return checkout
        private = (checkout / line[len("gitdir:"):].strip()).resolve()
        common = (private / (private / "commondir").read_text(encoding="utf-8").strip()).resolve()
        back = (private / (private / "gitdir").read_text(encoding="utf-8").strip()).resolve()
        if private.parent != common / "worktrees" or back != (checkout / ".git").resolve():
            return checkout
    except (OSError, UnicodeError, ValueError):
        return checkout
    return common.parent if common.name == ".git" else common


def project_key(start: str) -> str:
    """The folder Claude Code files a session's project settings under in
    its .claude.json - the dialog's answer about imports from outside the
    session's folder among them: the Git checkout's (the main one's, for a
    linked working tree), or the folder the session started in outside Git."""
    folder = Path(start)
    for candidate in (folder, *folder.parents):
        git = candidate / ".git"
        if git.is_symlink():
            break
        if git.is_dir():
            return str(candidate)
        if git.is_file():
            return str(main_checkout(candidate))
    return start


def external_imports_approved(start: str) -> bool:
    folder = os.environ.get("CLAUDE_CONFIG_DIR")
    config = Path(folder) / ".claude.json" if folder else Path.home() / ".claude.json"
    try:
        projects = json.loads(config.read_text(encoding="utf-8")).get("projects") or {}
    except (OSError, ValueError, AttributeError):
        return False
    entry = projects.get(project_key(start)) if isinstance(projects, dict) else None
    return isinstance(entry, dict) and entry.get("hasClaudeMdExternalIncludesApproved") is True


def width(text: str) -> int:
    return len(text.encode("utf-16-le")) // 2


def pieces(text: str, budget: int) -> list:
    """`text` cut at line ends into pieces of at most `budget` characters, a
    piece ending before the last heading outside a code block that starts in
    its second half when there is one."""
    result, lines, size = [], [], 0
    fenced = False
    for line in text.splitlines(keepends=True):
        heading = not fenced and line.startswith("#")
        if line.startswith("```"):
            fenced = not fenced
        while width(line) > budget:
            if lines:
                result.append("".join(earlier for earlier, _ in lines))
                lines, size = [], 0
            cut = budget
            while cut > 1 and width(line[:cut]) > budget:
                cut -= 1
            result.append(line[:cut])
            line, heading = line[cut:], False
        while lines and size + width(line) > budget:
            cut, prefix = len(lines), 0
            for index, (earlier, starts) in enumerate(lines):
                if index and starts and prefix >= budget // 2:
                    cut = index
                prefix += width(earlier)
            result.append("".join(earlier for earlier, _ in lines[:cut]))
            lines = lines[cut:]
            size = sum(width(earlier) for earlier, _ in lines)
        if line:
            lines.append((line, heading))
            size += width(line)
    if lines:
        result.append("".join(line for line, _ in lines))
    return result


def policy(root: str, start: str, part: int) -> str:
    """Part `part` of the project's AGENTS.md, with a line saying what it is;
    nothing past its end, nor when Claude Code imports the file itself."""
    path = Path(root) / "AGENTS.md"
    if not 1 <= part <= POLICY_PARTS or not path.is_file() or external_imports_approved(start):
        return ""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        return ""
    header = "Contents of {} (the project's policy), part {} of {}:\n\n"
    parts = pieces(text, INLINE - width(header.format(path, 99, 99)) - 64)
    count = min(len(parts), POLICY_PARTS)
    if part > count:
        return ""
    body = "".join(parts[part - 1:]) if part == POLICY_PARTS else parts[part - 1]
    return header.format(path, part, count) + body


def main() -> int:
    hook, root, start = sys.argv[1:4]
    if hook.startswith("policy-"):
        number = hook[len("policy-"):]
        # UTF-8 whatever the locale: a policy's dash fails an ASCII one.
        sys.stdout.buffer.write(policy(root, start, int(number) if number.isdigit() else 0).encode("utf-8"))
        return 0
    payload = sys.stdin.buffer.read()
    done = subprocess.run(["bash", str(Path(root) / ".claude/hooks" / hook)], input=payload, cwd=root,
                          stdout=subprocess.PIPE)
    if hook == "bash-validator.sh":
        sys.stdout.buffer.write(done.stdout)
        if done.returncode != 0 or done.stdout.strip():
            return done.returncode
        try:
            command = (json.loads(payload).get("tool_input") or {}).get("command")
        except (ValueError, AttributeError):
            return 0
        if memory_command(command, root):
            print(json.dumps({"hookSpecificOutput": {
                "hookEventName": "PreToolUse", "permissionDecision": "allow",
                "permissionDecisionReason": "the accelerator's memory command, as the project allows it"}}))
        return 0
    sys.stdout.buffer.write(done.stdout)
    return done.returncode


if __name__ == "__main__":
    sys.exit(main())
