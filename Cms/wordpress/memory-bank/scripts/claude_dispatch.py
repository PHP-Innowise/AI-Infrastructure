"""AI accelerator, Claude Code: what a session started below the project root
needs beyond the output of the project's own hooks (see
ai-accelerator-dispatch.sh beside this file).

bash-validator.sh: the guard runs as it does in the project root. A command
that then runs nothing but this project's memory CLI - the save step's
command among them - gets the allow the project's permission rules give it
in the root, which such a session does not read, so it is not asked about
the save after every answer.

permissions: the decision the project's permission rules - .claude/
settings.json and settings.local.json - make in the root: a deny or an ask
wherever they would, and an allow only for a file inside the root or a
single plain command, so a session below the root is never let further than
one in the root. Claude Code reads settings.json only in the root; it reads
settings.local.json below it too, but resolves its paths from the session's
folder, which is stricter there and never looser.

environment: the project's `env` for the session's commands, through the
file Claude Code names in CLAUDE_ENV_FILE at a session's start; the
CLAUDE_ and ANTHROPIC_ ones set Claude Code itself, which no hook can.

policy-N: part N of the project's AGENTS.md. Claude Code reaches it from
.claude/CLAUDE.md through an import from outside the session's folder, which
it asks about in a terminal and leaves out under -p. It keeps a hook's output
in the conversation only up to INLINE characters and puts a longer one in a
file, of which the model sees the first 2,000 - so the policy comes in parts,
one settings entry each, cut between sections.

A session in a worktree of the project's repository, which has none of an
installation Git does not track, is served the same way for the worktree
(PROJECT, the root's folder there): the root's rules decide for the files of
the worktree, its guard runs there, environment also gives the session's
commands the three variables that make the root's memory CLI serve the
worktree (workspace_roots.py), and the policy comes whatever Claude Code was
told about imports, since the worktree has nothing for it to import.

Usage: ai-accelerator-dispatch.py HOOK ROOT START [PROJECT], the hook's input
on stdin.
"""
import json
import os
import re
import shlex
import sys
from pathlib import Path

# shutil, subprocess and urllib.parse are imported where they are used: the
# permissions entry runs before every tool call below the root or in a
# worktree, and a decision needs none of them - a third of its start.

WINDOWS = os.name == "nt"

# One copy serves every project of the person: a session in the root of a
# project with an older release must not put an older one back. Raise it,
# with the comment in claude_dispatch.sh, whenever either file changes.
DISPATCH_VERSION = 4
INTERPRETERS = (["python3"], ["python"], ["py", "-3"])
CLI = "memory-bank/scripts/context.py"
# How .claude/CLAUDE.md, or the project's own CLAUDE.md, pulls AGENTS.md in.
POLICY_IMPORT = re.compile(r"(?:^|\s)@(?:\./|\.\./)?AGENTS\.md(?:\s|$)", re.M)
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


def native_path(path: str) -> str:
    """Git Bash's /c/Users/me as Windows writes it, C:/Users/me."""
    if WINDOWS and re.match(r"/[A-Za-z](/|$)", path):
        return path[1] + ":" + (path[2:] or "/")
    return path


def same_path(first: str, second: str) -> bool:
    def key(path):
        return os.path.normcase(os.path.normpath(native_path(path)))
    return key(first) == key(second)


def hook_bash() -> str:
    """The bash Claude Code runs hooks with: Git for Windows' on Windows,
    where a bare bash is WSL's launcher, which CreateProcess finds first."""
    import shutil
    if not WINDOWS:
        return shutil.which("bash") or "bash"
    configured = os.environ.get("CLAUDE_CODE_GIT_BASH_PATH")
    if configured and Path(configured).is_file():
        return configured
    git = shutil.which("git")
    for folder in Path(git).resolve().parents if git else ():
        if (folder / "bin" / "bash.exe").is_file():
            return str(folder / "bin" / "bash.exe")
    return shutil.which("bash") or "bash"


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
        if len(words) < 3 or not same_path(words[1], root) or words[2] != "&&":
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
    if not script or not (same_path(script, os.path.join(root, CLI)) or relative and script == CLI):
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


def entered_worktree(root: str, cwd: str) -> bool:
    """Whether `cwd` is in a worktree of `root`'s repository that has none of
    its installation - where a session started in the root works once it has
    entered the worktree (Claude Code's EnterWorktree), served there by the
    root's own hooks."""
    try:
        folder = Path(native_path(cwd)).resolve()
        top = next((candidate for candidate in (folder, *folder.parents) if (candidate / ".git").exists()), None)
        if top is None or not (top / ".git").is_file():
            return False
        main = main_checkout(top)
        if main == top:
            return False
        place = top / Path(native_path(root)).resolve().relative_to(main.resolve())
        folder.relative_to(place)
    except (OSError, ValueError):
        return False
    return place.is_dir() and not (place / "memory-bank" / "scripts" / "context.py").exists()


def project_key(start: str) -> str:
    """The folder Claude Code files a session's project settings under in
    its .claude.json - the dialog's answer about imports from outside the
    session's folder among them: the Git checkout's (the main one's, for a
    linked working tree), or the folder the session started in outside Git."""
    folder = Path(native_path(start))
    found = str(folder)
    for candidate in (folder, *folder.parents):
        git = candidate / ".git"
        if git.is_symlink():
            break
        if git.is_dir():
            found = str(candidate)
            break
        if git.is_file():
            found = str(main_checkout(candidate))
            break
    # Claude Code on Windows files it with forward slashes.
    return found.replace("\\", "/") if WINDOWS else found


def external_imports_approved(start: str) -> bool:
    folder = os.environ.get("CLAUDE_CONFIG_DIR")
    config = Path(folder) / ".claude.json" if folder else Path.home() / ".claude.json"
    try:
        projects = json.loads(config.read_text(encoding="utf-8")).get("projects") or {}
    except (OSError, ValueError, AttributeError):
        return False
    if not isinstance(projects, dict):
        return False
    key = project_key(start)
    entry = projects.get(key)
    if entry is None and WINDOWS:
        entry = next((value for name, value in projects.items() if name.lower() == key.lower()), None)
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


def imports_policy(root: str) -> bool:
    """Whether the project's CLAUDE.md pulls AGENTS.md into a session in the
    root: below it the dispatcher stands in for that, and for nothing else."""
    for name in (".claude/CLAUDE.md", "CLAUDE.md"):
        try:
            if POLICY_IMPORT.search((Path(root) / name).read_text(encoding="utf-8")):
                return True
        except (OSError, UnicodeError):
            continue
    return False


def policy(root: str, start: str, part: int, project: str = "") -> str:
    """Part `part` of the project's AGENTS.md, with a line saying what it is;
    nothing past its end, nor where the project's CLAUDE.md does not import
    it, nor when Claude Code imports it itself - which in a worktree without
    the installation it cannot, unless the worktree has its own AGENTS.md."""
    path = Path(root) / "AGENTS.md"
    if not 1 <= part <= POLICY_PARTS or not path.is_file() or not imports_policy(root):
        return ""
    if (Path(project) / "AGENTS.md").is_file() if project else external_imports_approved(start):
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


# The tools a Read or an Edit rule covers, with the input naming the file.
# Claude Code 2.1 refuses to write a file a Read rule denies, too.
READ_TOOLS = {"Read": "file_path"}
EDIT_TOOLS = {"Edit": "file_path", "Write": "file_path", "MultiEdit": "file_path", "NotebookEdit": "notebook_path"}
OPERATORS = {"&&", "||", ";", "|", "&", "|&", ";;", "(", ")", "\n"}
# What Claude Code 2.1 looks through to the command a deny rule names, read
# from the binary and checked by differential runs against it: `env rm -rf
# x`, `... | xargs rm -rf`, `sudo rm -rf x` and `if ...; then rm -rf x; fi`
# are refused under Bash(rm -rf:*) in the root, while `command -v sudo`,
# `xargs -I{} rm -rf {}`, `busybox rm -rf x`, `coproc rm -rf x` and a
# wrapper named by its path (`/usr/bin/env rm -rf x`) are not.
WRAPPERS = {"time", "nohup", "timeout", "nice", "stdbuf", "env", "command", "builtin", "noglob", "xargs",
            "sudo", "doas", "exec", "setsid", "ionice", "pkexec", "runuser", "eval"}
KEYWORDS = {"if", "then", "else", "elif", "do", "while", "until", "!", "{", "}"}
ASSIGNMENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\+?=.*")
# Options of a wrapper that take the next word as their value.
VALUES = {"env": {"-u"}, "sudo": {"-u", "-g", "-C", "-h", "-p", "-U", "-r", "-t"}, "doas": {"-u", "-C"},
          "timeout": {"-k", "-s"}, "runuser": {"-u", "-g"}}


def project_rules(root: str) -> dict:
    """The allow, ask and deny rules of the project's shared and local
    settings, as Claude Code merges them for a session in the root."""
    rules = {"allow": [], "ask": [], "deny": []}
    for name in ("settings.json", "settings.local.json"):
        try:
            data = json.loads((Path(root) / ".claude" / name).read_text(encoding="utf-8"))
        except (OSError, ValueError, UnicodeError):
            continue
        permissions = data.get("permissions") if isinstance(data, dict) else None
        for kind in rules:
            values = permissions.get(kind) if isinstance(permissions, dict) else None
            rules[kind] += [value for value in values if isinstance(value, str)] if isinstance(values, list) else []
    return rules


def split_rule(rule: str):
    """(tool, content) of `Tool(content)`; content None for a bare `Tool`."""
    found = re.fullmatch(r"\s*([A-Za-z_][A-Za-z0-9_-]*)(?:\((.*)\))?\s*", rule, re.S)
    return (found.group(1), found.group(2)) if found else (None, None)


def glob_regex(glob: str) -> str:
    parts, index = [], 0
    while index < len(glob):
        if glob.startswith("**/", index):
            parts.append("(?:.*/)?")
            index += 3
        elif glob.startswith("**", index):
            parts.append(".*")
            index += 2
        else:
            parts.append({"*": "[^/]*", "?": "[^/]"}.get(glob[index], re.escape(glob[index])))
            index += 1
    return "".join(parts)


def path_rule(content: str, root: str):
    """(base, regex, anchored, folder) of a Read or Edit rule's path, as
    Claude Code 2.1 reads one in the project root: gitignore patterns from
    the root, `/x` from the root, `//x` absolute, `~/x` from home. A pattern
    with a / before its end is anchored to its base; a trailing `/` or
    `/**` names a folder and is not one, so `secrets/**` matches a secrets
    folder at any depth while `a/b/**` matches only the root's a/b."""
    text = content.strip()
    if text.startswith("//") and WINDOWS and re.match(r"//[A-Za-z](/|$)", text):
        base, text, anchored = text[2] + ":" + os.sep, text[4:], True
    elif text.startswith("//"):
        base, text, anchored = os.sep, text[2:], True
    elif text.startswith("~/"):
        base, text, anchored = str(Path.home()), text[2:], True
    elif text.startswith("/"):
        base, text, anchored = root, text[1:], True
    else:
        base, anchored = root, None
        if text.startswith("./"):
            text = text[2:]
    folder = False
    if text.endswith("/**"):
        text, folder = text[:-3], True
    elif text.endswith("/"):
        text, folder = text.rstrip("/"), True
    if not text:
        text, folder = "**", False
    if anchored is None:
        anchored = "/" in text
    return base, re.compile(glob_regex(text), re.IGNORECASE if WINDOWS else 0), anchored, folder


def path_matches(content: str, path: str, root: str) -> bool:
    """Whether the rule's path names `path` (absolute) or a folder holding it."""
    base, regex, anchored, folder = path_rule(content, root)
    try:
        relative = os.path.relpath(path, base) if base != os.sep else path.lstrip(os.sep)
    except ValueError:  # another drive
        return False
    if relative == "." or relative.startswith(".." + os.sep) or relative == ".." or os.path.isabs(relative):
        return False
    names = relative.replace(os.sep, "/").split("/")
    for depth in range(1, len(names) + 1):
        candidate = "/".join(names[:depth])
        is_folder = depth < len(names) or os.path.isdir(path)
        if folder and not is_folder:
            continue
        if regex.fullmatch(candidate if anchored else names[depth - 1]):
            return True
    return False


def file_paths(tool_input: dict, key: str, start: str) -> list:
    """The file a tool names, as written and with links resolved."""
    value = tool_input.get(key)
    if not isinstance(value, str) or not value or "\0" in value:
        return []
    value = native_path(value)
    path = os.path.normpath(value if os.path.isabs(value) else os.path.join(native_path(start), value))
    return list(dict.fromkeys([path, os.path.realpath(path)]))


def inside(path: str, root: str) -> bool:
    path, root = os.path.normcase(path), os.path.normcase(os.path.normpath(native_path(root)))
    return path == root or path.startswith(root.rstrip(os.sep) + os.sep)


def words(command: str):
    try:
        lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
        lexer.whitespace_split = True
        return list(lexer)
    except ValueError:
        return None


def segments(command: str) -> list:
    """The simple commands of `command`, each a list of words without its
    leading variable assignments; those inside $(...) and backticks too."""
    found = []
    tokens = words(command.replace("\n", " ; "))
    if tokens is None:
        tokens = re.split(r"\s+|([;&|()])", command)
        tokens = [token for token in tokens if token]
    current = []
    for token in tokens + [";"]:
        punctuation = bool(token) and set(token) <= set("();<>|&")
        if token in OPERATORS or punctuation and set(token) & set(";&|()"):
            while current and ASSIGNMENT.fullmatch(current[0]):
                current = current[1:]
            if current:
                found.append(current)
            current = []
        elif not punctuation:
            # A redirection's < or > is left out; its target stays a word.
            current.append(token)
    for inner in re.findall(r"\$\(([^()]*)\)|`([^`]*)`", command):
        found += segments(inner[0] or inner[1])
    return found


def forms(segment: list):
    """The segment and each command it runs as keywords, assignments and
    wrappers with their options come off - a rule may name the wrapper
    itself (Bash(sudo:*)) or what it runs (Bash(rm -rf:*)) - but through a
    bare `xargs` only, and not through `command -v`."""
    yield segment
    while segment:
        word = segment[0]
        if word in KEYWORDS or ASSIGNMENT.fullmatch(word):
            segment = segment[1:]
            yield segment
            continue
        name = word
        if name not in WRAPPERS:
            return
        rest = segment[1:]
        if name == "xargs" and (not rest or rest[0].startswith("-")):
            return
        if name == "command" and any("v" in option or "V" in option for option in rest[:1] if option.startswith("-")):
            return
        while rest and (rest[0].startswith("-") or re.fullmatch(r"[0-9.]+[smhd]?", rest[0])
                        or name == "env" and ASSIGNMENT.fullmatch(rest[0])):
            option, rest = rest[0], rest[1:]
            if option in VALUES.get(name, ()) and rest:
                rest = rest[1:]
        segment = rest
        yield segment


def bash_matches(content: str, segment: list) -> bool:
    if content.endswith(":*"):
        prefix = words(content[:-2].strip())
        return bool(prefix) and segment[:len(prefix)] == prefix
    if "*" in content:
        pattern = "".join(".*" if character == "*" else re.escape(character) for character in content.strip())
        return re.fullmatch(pattern, " ".join(segment)) is not None
    return " ".join(segment) == " ".join(content.split())


def domain_matches(content: str, url: str) -> bool:
    if not content.startswith("domain:"):
        return False
    from urllib.parse import urlsplit
    domain, host = content[len("domain:"):].strip().lower(), (urlsplit(url).hostname or "").lower()
    if domain.startswith("*."):
        return host.endswith(domain[1:])
    return bool(host) and host == domain


def matches(rule: str, tool: str, tool_input: dict, root: str, start: str, allow: bool) -> bool:
    """Whether `rule` covers this call; for an allow, only where nothing it
    does could reach further than the same call in the root."""
    rule_tool, content = split_rule(rule)
    if rule_tool is None:
        return False
    if content is None:
        return rule_tool == tool or (rule_tool.startswith("mcp__") and tool.startswith(rule_tool + "__"))
    if rule_tool in ("Read", "Edit") and (tool in READ_TOOLS or tool in EDIT_TOOLS):
        if rule_tool == "Edit" and tool not in EDIT_TOOLS:
            return False
        if rule_tool == "Read" and tool in EDIT_TOOLS and allow:
            return False
        paths = file_paths(tool_input, READ_TOOLS.get(tool) or EDIT_TOOLS[tool], start)
        if allow:
            return bool(paths) and all(inside(path, root) and path_matches(content, path, root) for path in paths)
        return any(path_matches(content, path, root) for path in paths)
    if rule_tool == "Bash" and tool == "Bash":
        command = tool_input.get("command")
        if not isinstance(command, str):
            return False
        if allow:
            # One plain command, as the root's rule would let through.
            parts = segments(command)
            return plain(command) and len(parts) == 1 and bash_matches(content, parts[0])
        return any(bash_matches(content, form) for segment in segments(command) for form in forms(segment))
    if rule_tool in ("Agent", "Task") and tool in ("Agent", "Task") and not allow:
        return tool_input.get("subagent_type") == content.strip()
    if rule_tool == "WebFetch" and tool == "WebFetch":
        url = tool_input.get("url")
        return isinstance(url, str) and domain_matches(content, url)
    return False


def own_rules() -> list:
    """The deny and ask rules of the person's own and the managed settings,
    which Claude Code applies below the root by itself."""
    folder = os.environ.get("CLAUDE_CONFIG_DIR")
    sources = [Path(folder) / "settings.json" if folder else Path.home() / ".claude" / "settings.json",
               Path("/etc/claude-code/managed-settings.json"),
               Path("/Library/Application Support/ClaudeCode/managed-settings.json"),
               Path(os.environ.get("ProgramFiles") or "C:\\Program Files") / "ClaudeCode" / "managed-settings.json"]
    found = []
    for source in sources:
        try:
            data = json.loads(source.read_text(encoding="utf-8"))
        except (OSError, ValueError, UnicodeError):
            continue
        permissions = data.get("permissions") if isinstance(data, dict) else None
        for kind in ("deny", "ask"):
            values = permissions.get(kind) if isinstance(permissions, dict) else None
            found += [value for value in values if isinstance(value, str)] if isinstance(values, list) else []
    return found


def permission(payload: dict, root: str, start: str, project: str = ""):
    """The project's decision for this tool call in the root - deny, then
    ask, then allow - as a PreToolUse answer, or None. An allow is never
    given where the person's own or the managed settings deny or ask. In a
    worktree the root's rules decide for the worktree's files as for the
    root's, and a deny or an ask also covers the root's own files there."""
    tool, tool_input = payload.get("tool_name"), payload.get("tool_input")
    if not isinstance(tool, str) or not isinstance(tool_input, dict):
        return None
    rules = project_rules(root)
    place = project or root
    places = (project, root) if project else (root,)
    for kind in ("deny", "ask", "allow"):
        for rule in rules[kind]:
            if kind == "allow":
                found = matches(rule, tool, tool_input, place, start, allow=True)
            else:
                found = any(matches(rule, tool, tool_input, base, start, allow=False) for base in places)
            if found:
                if kind == "allow" and any(matches(own, tool, tool_input, base, start, allow=False)
                                           for own in own_rules() for base in (*places, start)):
                    return None
                answer = {"hookEventName": "PreToolUse", "permissionDecision": kind}
                if kind == "deny":
                    answer["permissionDecisionReason"] = (
                        f"The project's settings deny this ({rule}), as they would in the project root.")
                return {"hookSpecificOutput": answer}
    return None


def environment(root: str, project: str = "") -> list:
    """`export` lines for the project's env, shared settings then local; in
    a worktree also the variables that make the root's memory CLI - the save
    step's - serve the worktree, as its hooks do."""
    values = {}
    for name in ("settings.json", "settings.local.json"):
        try:
            data = json.loads((Path(root) / ".claude" / name).read_text(encoding="utf-8"))
        except (OSError, ValueError, UnicodeError):
            continue
        env = data.get("env") if isinstance(data, dict) else None
        for key, value in (env.items() if isinstance(env, dict) else ()):
            if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key) and not key.startswith(("CLAUDE_", "ANTHROPIC_")) \
                    and isinstance(value, (str, int, float, bool)):
                values[key] = str(value).lower() if isinstance(value, bool) else str(value)
    if project:
        values.update(ACCELERATOR_HOME=root, ACCELERATOR_STATE_DIR=root, ACCELERATOR_PROJECT_DIR=project)
    return [f"export {key}={shlex.quote(value)}" for key, value in values.items()]


def main() -> int:
    hook, root, start = sys.argv[1:4]
    project = sys.argv[4] if len(sys.argv) > 4 else ""
    if hook == "entered":
        # A session started in the root that entered a worktree of it: the
        # root's own guard has run; the memory CLI gets the allow it has in
        # the root, which the save step its hooks ask for needs there.
        try:
            payload = json.loads(sys.stdin.buffer.read() or b"{}")
        except ValueError:
            return 0
        if isinstance(payload, dict) and isinstance(payload.get("cwd"), str) and entered_worktree(root, payload["cwd"]):
            if memory_command((payload.get("tool_input") or {}).get("command"), root):
                print(json.dumps({"hookSpecificOutput": {
                    "hookEventName": "PreToolUse", "permissionDecision": "allow",
                    "permissionDecisionReason": "the accelerator's memory command, as the project allows it"}}))
        return 0
    if hook == "environment":
        target = os.environ.get("CLAUDE_ENV_FILE")
        lines = environment(root, project)
        if target and lines:
            with open(target, "a", encoding="utf-8") as handle:
                handle.write("\n".join(lines) + "\n")
        return 0
    if hook == "permissions":
        try:
            payload = json.loads(sys.stdin.buffer.read() or b"{}")
        except ValueError:
            return 0
        answer = permission(payload, root, start, project) if isinstance(payload, dict) else None
        if answer:
            print(json.dumps(answer))
        return 0
    if hook.startswith("policy-"):
        number = hook[len("policy-"):]
        # UTF-8 whatever the locale: a policy's dash fails an ASCII one.
        sys.stdout.buffer.write(policy(root, start, int(number) if number.isdigit() else 0, project).encode("utf-8"))
        return 0
    payload = sys.stdin.buffer.read()
    import subprocess
    done = subprocess.run([hook_bash(), str(Path(root) / ".claude/hooks" / hook)], input=payload,
                          cwd=project or root, stdout=subprocess.PIPE)
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
