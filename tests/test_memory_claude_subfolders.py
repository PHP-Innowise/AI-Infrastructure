"""Claude Code started in a folder below an installed project.

Claude Code reads a project's settings - its hooks and permission rules -
only from the folder a session starts in, so a session started in src/ ran
none of the project's hooks, and the project's AGENTS.md, which
.claude/CLAUDE.md imports from outside that folder, came only through a
dialog. A session in the project root keeps a dispatcher in the person's own
Claude Code settings (memory-bank/scripts/claude_subfolders.py); the
dispatcher runs the project's hooks for a session started below the root,
brings the policy in parts Claude Code keeps in the conversation, and lets
the memory CLI through as the project's own permission rules do.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

from tests.hook_shell import BASH

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = os.name == "nt"
EDITIONS = ("PHP Core", "Laravel", "Symfony", "Cms/wordpress")
SCRIPTS = ROOT / "PHP Core" / "memory-bank" / "scripts"


def load_dispatch():
    spec = importlib.util.spec_from_file_location("claude_dispatch_under_test", SCRIPTS / "claude_dispatch.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


dispatch = load_dispatch()


def shown(path):
    """`path` as Git Bash shows it, for comparison: C:\\Users reads /c/Users
    there, and Windows names ignore case."""
    text = str(path)
    if WINDOWS:
        text = text.replace("\\", "/")
        if len(text) > 1 and text[1] == ":":
            text = "/" + text[0] + text[2:]
        text = text.lower()
    return text


def key_of(path):
    """The key Claude Code files a project under in its .claude.json."""
    return str(path).replace("\\", "/") if WINDOWS else str(path)


def can_link(folder):
    """Whether this machine lets a test make a symbolic link (Windows needs a privilege)."""
    probe = Path(folder) / "link-probe"
    try:
        probe.symlink_to(folder, target_is_directory=True)
    except (OSError, NotImplementedError):
        return False
    probe.unlink()
    return True


def inside_git(folder):
    """Whether a folder above `folder` holds a .git (a machine's /tmp can)."""
    return any((parent / ".git").exists() for parent in Path(folder).parents)


def git(folder, *args):
    subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                    "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
                   cwd=folder, check=True, capture_output=True)


@unittest.skipIf(BASH is None, "no bash to run hooks with")
class Fixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="claude-subfolders-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.config = self.base / "config"
        (self.config / "projects" / "-project").mkdir(parents=True)
        self.transcript = self.config / "projects" / "-project" / "session.jsonl"
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("CLAUDE", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        self.env.update(HOME=str(self.base / "home"), CLAUDE_CONFIG_DIR=str(self.config),
                        PYTHONDONTWRITEBYTECODE="1")
        (self.base / "home").mkdir()

    def project(self, name="project", hooks=None, policy=None):
        """A project with the edition's Claude settings and the dispatcher's
        scripts; `hooks` maps a hook name to its script."""
        root = self.base / name
        (root / "memory-bank" / "scripts").mkdir(parents=True)
        (root / ".claude" / "hooks").mkdir(parents=True)
        (root / "src" / "Billing").mkdir(parents=True)
        shutil.copy2(ROOT / "PHP Core" / ".claude" / "settings.json", root / ".claude" / "settings.json")
        for script in ("claude_dispatch.py", "claude_dispatch.sh", "claude_subfolders.py"):
            shutil.copy2(SCRIPTS / script, root / "memory-bank" / "scripts" / script)
        (root / "memory-bank" / "scripts" / "context.py").write_text("# the memory CLI\n", encoding="utf-8")
        for hook, body in (hooks or {}).items():
            path = root / ".claude" / "hooks" / hook
            path.write_text("#!/bin/bash\n" + body, encoding="utf-8")
            path.chmod(0o755)
        if policy is not None:
            (root / "AGENTS.md").write_text(policy, encoding="utf-8")
            shutil.copy2(ROOT / "PHP Core" / ".claude" / "CLAUDE.md", root / ".claude" / "CLAUDE.md")
        git(root, "init", "-q")
        return root

    def setup_dispatcher(self, root, transcript=None, check=True):
        payload = {"session_id": "s", "transcript_path": str(transcript or self.transcript),
                   "hook_event_name": "SessionStart", "source": "startup"}
        done = subprocess.run([sys.executable, str(root / "memory-bank" / "scripts" / "claude_subfolders.py")],
                              input=json.dumps(payload).encode(), env=self.env, cwd=root, capture_output=True)
        if check:
            self.assertEqual(done.returncode, 0, done.stderr)
        return done

    def settings(self):
        return json.loads((self.config / "settings.json").read_text(encoding="utf-8"))

    def dispatched(self, settings=None):
        """(event, matcher, hook name, timeout) of every dispatcher entry."""
        marker = shlex.quote((self.config / "ai-accelerator" / "ai-accelerator-dispatch.sh").as_posix()) + " "
        found = []
        for event, groups in (settings or self.settings()).get("hooks", {}).items():
            for group in groups:
                for hook in group["hooks"]:
                    if hook.get("command", "").startswith(marker):
                        found.append((event, group.get("matcher", ""), hook["command"][len(marker):],
                                      hook.get("timeout")))
        return found

    def run_dispatcher(self, hook, start, payload=b"{}"):
        env = dict(self.env, CLAUDE_PROJECT_DIR=str(start))
        # As the settings entry names it: forward slashes, which Git Bash
        # on Windows runs as written.
        return subprocess.run([BASH, (self.config / "ai-accelerator" / "ai-accelerator-dispatch.sh").as_posix(), hook],
                              input=payload, env=env, cwd=start, capture_output=True)

    def assert_ran(self, done, root, payload="{}"):
        """The echo hook ran in `root`, with CLAUDE_PROJECT_DIR naming it."""
        self.assertEqual(done.returncode, 0, done.stderr)
        where, _, rest = done.stdout.decode().partition(" for ")
        named, _, given = rest.partition(": ")
        self.assertEqual((shown(where[len("ran in "):]), named, given), (shown(root), str(root), payload), done.stdout)


class PolicyPieces(unittest.TestCase):
    def test_every_edition_policy_is_cut_between_sections_within_the_limit(self):
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                text = (ROOT / edition / "AGENTS.md").read_text(encoding="utf-8")
                parts = dispatch.pieces(text, 9_000)
                self.assertEqual("".join(parts), text)
                self.assertGreater(len(parts), 1, "the policy is longer than one hook output keeps")
                self.assertLessEqual(len(parts), dispatch.POLICY_PARTS)
                for part in parts:
                    self.assertLessEqual(dispatch.width(part), 9_000)
                for part in parts[1:]:
                    self.assertTrue(part.startswith("#"), part[:80])

    def test_a_comment_in_a_code_block_is_not_a_section(self):
        text = "# Policy\n" + "rule\n" * 30 + "```bash\n# a comment\n" + "run\n" * 30 + "```\n"
        parts = dispatch.pieces(text, 200)
        self.assertEqual("".join(parts), text)
        self.assertFalse(any(part.startswith("# a comment") for part in parts))

    def test_a_line_longer_than_a_piece_and_characters_beyond_the_basic_plane(self):
        text = "# Head\n" + "\U0001F600" * 150 + "\nend\n"
        parts = dispatch.pieces(text, 64)
        self.assertEqual("".join(parts), text)
        self.assertTrue(all(0 < dispatch.width(part) <= 64 for part in parts))
        self.assertEqual(dispatch.pieces("", 64), [])


class PolicyParts(Fixture):
    def policy_text(self, sections):
        return "".join(f"## Section {number}\n" + f"Rule {number} holds.\n" * 300 for number in range(sections))

    def test_parts_carry_the_whole_policy_each_within_what_claude_code_keeps_inline(self):
        root = self.project(policy=self.policy_text(4))
        text = (root / "AGENTS.md").read_text(encoding="utf-8")
        with mock.patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": str(self.config)}):
            outputs = [dispatch.policy(str(root), str(root / "src"), part)
                       for part in range(1, dispatch.POLICY_PARTS + 2)]
        given = [output for output in outputs if output]
        self.assertGreater(len(given), 1)
        self.assertEqual(outputs[len(given):], [""] * (len(outputs) - len(given)))
        bodies = []
        for number, output in enumerate(given, 1):
            header = f"Contents of {root / 'AGENTS.md'} (the project's policy), part {number} of {len(given)}:\n\n"
            self.assertTrue(output.startswith(header), output[:200])
            self.assertLessEqual(dispatch.width(output.strip()), dispatch.INLINE)
            bodies.append(output[len(header):])
        self.assertEqual("".join(bodies), text)

    def test_a_policy_longer_than_the_entries_ends_whole_in_the_last(self):
        root = self.project(policy=self.policy_text(40))
        text = (root / "AGENTS.md").read_text(encoding="utf-8")
        with mock.patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": str(self.config)}):
            outputs = [dispatch.policy(str(root), str(root / "src"), part)
                       for part in range(1, dispatch.POLICY_PARTS + 1)]
        last = f"part {dispatch.POLICY_PARTS} of {dispatch.POLICY_PARTS}:\n\n"
        self.assertIn(last, outputs[-1])
        self.assertTrue(text.endswith(outputs[-1].split(last, 1)[1]))
        self.assertEqual("".join(output.split(":\n\n", 1)[1] for output in outputs), text)

    def approve(self, folder):
        (self.config / ".claude.json").write_text(json.dumps({"projects": {key_of(folder): {
            "hasClaudeMdExternalIncludesApproved": True, "hasClaudeMdExternalIncludesWarningShown": True}}}),
            encoding="utf-8")

    def test_nothing_when_claude_code_imports_the_policy_itself_or_there_is_none(self):
        root = self.project(policy="# Policy\nRule.\n")
        start = root / "src" / "Billing"
        with mock.patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": str(self.config)}):
            # Claude Code files the dialog's answer under the checkout, not
            # under the folder the session started in.
            self.approve(start)
            self.assertTrue(dispatch.policy(str(root), str(start), 1))
            self.approve(root)
            self.assertEqual(dispatch.policy(str(root), str(start), 1), "")
            self.assertEqual(dispatch.policy(str(root), str(root / "src"), 1), "")
            (self.config / ".claude.json").unlink()
            self.assertTrue(dispatch.policy(str(root), str(start), 1))
            (root / "AGENTS.md").unlink()
            self.assertEqual(dispatch.policy(str(root), str(start), 1), "")
            self.assertEqual(dispatch.policy(str(root), str(start), 0), "")


class PolicyImport(Fixture):
    def test_the_policy_comes_only_where_the_projects_claude_md_imports_it(self):
        root = self.project(policy="# Policy\nRule.\n")
        start = str(root / "src")
        with mock.patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": str(self.config)}):
            self.assertTrue(dispatch.policy(str(root), start, 1))
            (root / ".claude" / "CLAUDE.md").write_text("The project keeps its policy to itself.\n", encoding="utf-8")
            self.assertEqual(dispatch.policy(str(root), start, 1), "")
            (root / "CLAUDE.md").write_text("# Team notes\n\n@AGENTS.md\n", encoding="utf-8")
            self.assertTrue(dispatch.policy(str(root), start, 1))
            (root / "CLAUDE.md").write_text("Mail me@AGENTS.md.example\n", encoding="utf-8")
            self.assertEqual(dispatch.policy(str(root), start, 1), "")


class DispatcherVersion(Fixture):
    # Raise DISPATCH_VERSION with any change to claude_dispatch.sh or .py,
    # then record the scripts' new fingerprint here.
    FINGERPRINTS = {3: "b521f764a0743b4fa164b3e89cc3024e0d420b1a776e568830ae57fb4c04f2cc"}

    def test_both_scripts_carry_the_version_and_a_change_raises_it(self):
        shell = (SCRIPTS / "claude_dispatch.sh").read_text(encoding="utf-8")
        self.assertIn(f"# dispatch-version: {dispatch.DISPATCH_VERSION} ", shell)
        digest = hashlib.sha256((SCRIPTS / "claude_dispatch.sh").read_bytes()
                                + (SCRIPTS / "claude_dispatch.py").read_bytes()).hexdigest()
        self.assertEqual(self.FINGERPRINTS.get(dispatch.DISPATCH_VERSION), digest,
                         "claude_dispatch changed: raise DISPATCH_VERSION and record this fingerprint")

    def test_a_newer_dispatcher_in_place_is_not_put_back(self):
        root = self.project(policy="# Policy\n")
        folder = self.config / "ai-accelerator"
        folder.mkdir(mode=0o700)
        newer = f"DISPATCH_VERSION = {dispatch.DISPATCH_VERSION + 1}\n"
        (folder / "ai-accelerator-dispatch.py").write_text(newer, encoding="utf-8")
        (folder / "ai-accelerator-dispatch.sh").write_text("# newer\n", encoding="utf-8")
        self.setup_dispatcher(root)
        self.assertEqual((folder / "ai-accelerator-dispatch.py").read_text(encoding="utf-8"), newer)
        self.assertEqual((folder / "ai-accelerator-dispatch.sh").read_text(encoding="utf-8"), "# newer\n")
        self.assertEqual((folder / "projects").read_text(encoding="utf-8").splitlines(), [str(root)])
        self.assertTrue(self.dispatched(), "the entries are added all the same")
        (folder / "ai-accelerator-dispatch.py").write_text("DISPATCH_VERSION = 1\n", encoding="utf-8")
        self.setup_dispatcher(root)
        self.assertEqual((folder / "ai-accelerator-dispatch.py").read_bytes(),
                         (SCRIPTS / "claude_dispatch.py").read_bytes(), "an older one is replaced")


class ProjectKey(Fixture):
    """Where Claude Code 2.1 files a session's project settings."""

    def test_the_checkout_outside_git_the_start_folder_and_a_linked_tree_its_main_checkout(self):
        root = self.project(policy="# Policy\n")
        self.assertEqual(dispatch.project_key(str(root / "src" / "Billing")), key_of(root))
        self.assertEqual(dispatch.project_key(str(root)), key_of(root))
        plain = self.base / "plain" / "src"
        plain.mkdir(parents=True)
        if not inside_git(self.base):
            self.assertEqual(dispatch.project_key(str(plain)), key_of(plain))
        git(root, "add", ".")
        git(root, "commit", "-qm", "base")
        linked = self.base / "linked"
        git(root, "worktree", "add", "-q", str(linked))
        (linked / "src").mkdir(exist_ok=True)
        self.assertEqual(dispatch.project_key(str(linked / "src")), key_of(root))
        # A .git file that names no linked tree of a repository counts as is.
        forged = self.base / "forged"
        (forged / "src").mkdir(parents=True)
        (forged / ".git").write_text(f"gitdir: {root / '.git'}\n", encoding="utf-8")
        self.assertEqual(dispatch.project_key(str(forged / "src")), key_of(forged))


class MemoryCommand(unittest.TestCase):
    ROOT = "/work/shop"

    def test_the_save_step_and_the_memory_cli_alone_are_recognised(self):
        cli = f"{self.ROOT}/memory-bank/scripts/context.py"
        for command in (
            f"cd {self.ROOT} && echo -E '{{\"progress\": \"it'\\''s done\"}}' | python3 memory-bank/scripts/context.py "
            f"--root {self.ROOT} native-result --request abc --token def --input - --json",
            f"python3 {cli} retrieve --query 'rollback DROP TABLE' --json",
            f"printf '%s' '{{}}' | python3 {cli} native-result --input - --json",
            f"py -3 {cli} status --json",
            f"cd {self.ROOT} && python memory-bank/scripts/context.py update --json",
        ):
            with self.subTest(command=command):
                self.assertTrue(dispatch.memory_command(command, self.ROOT))

    def test_anything_more_is_left_to_the_permission_prompt(self):
        cli = f"{self.ROOT}/memory-bank/scripts/context.py"
        for command in (
            f"python3 {cli} status; rm -rf /",
            f"python3 {cli} status && curl example.invalid",
            f"python3 {cli} status | sh",
            f"python3 {cli} status > /etc/passwd",
            f"python3 {cli} status $(id)",
            f"python3 {cli} status `id`",
            f'python3 {cli} retrieve --query "$(id)"',
            f"python3 {cli} status\nid",
            f"echo '{{}}' ; id | python3 {cli} native-result --input -",
            f"cat secrets | python3 {cli} native-result --input -",
            f"cd /elsewhere && python3 memory-bank/scripts/context.py status",
            "python3 memory-bank/scripts/context.py status",
            f"python3 /other/memory-bank/scripts/context.py status",
            f"python3 {cli} retrieve --query 'unbalanced",
            f"bash -c 'python3 {cli} status'",
            "",
        ):
            with self.subTest(command=command):
                self.assertFalse(dispatch.memory_command(command, self.ROOT))
        self.assertFalse(dispatch.memory_command(None, self.ROOT))


class Permissions(Fixture):
    """The project's permission rules below the root, as Claude Code 2.1.278
    applies them in the root (probed offline with the real binary)."""

    def test_path_patterns_match_as_claude_code_matches_them_in_the_root(self):
        root = self.base / "shop"
        home = Path.home()
        cases = {
            # rule: (paths it denies, paths it does not), relative to the root
            "./alpha/**": (["alpha/a.txt", "sub/alpha/a.txt"], ["alpha.txt"]),
            "zeta/**": (["zeta/z.txt", "sub/zeta/z.txt"], []),
            "h1*/**": (["h1z/x.txt", "s/h1z/x.txt"], []),
            "c2/": (["c2/x.txt", "s/c2/x.txt"], []),
            "beta/x.txt": (["beta/x.txt"], ["sub/beta/x.txt"]),
            "/gamma/**": (["gamma/g.txt"], ["sub/gamma/g.txt"]),
            "a1/b1/**": (["a1/b1/x.txt"], ["s/a1/b1/x.txt"]),
            "c1/*": (["c1/x.txt", "c1/n/x.txt"], ["s/c1/x.txt"]),
            "./d1/e1.txt": (["d1/e1.txt"], ["s/d1/e1.txt"]),
            "f1/**/g1.txt": (["f1/q/g1.txt"], ["s/f1/q/g1.txt"]),
            "delta": (["delta/d.txt", "sub/delta/d.txt"], []),
            "**/eps.txt": (["eps.txt", "sub/eps.txt"], []),
            "*.key": (["a.key", "sub/b.key"], ["a.keys"]),
            ".env": ([".env", "src/.env"], [".envrc"]),
            ".env.*": ([".env.local"], [".env"]),
            "./secrets/**": (["secrets/key.txt", "src/secrets/key.txt"], ["secrets.txt"]),
            "**": (["composer.json", "src/Invoice.php"], []),
        }
        for rule, (denied, kept) in cases.items():
            for relative in denied:
                with self.subTest(rule=rule, path=relative):
                    self.assertTrue(dispatch.path_matches(rule, str(root / relative), str(root)))
            for relative in kept:
                with self.subTest(rule=rule, path=relative):
                    self.assertFalse(dispatch.path_matches(rule, str(root / relative), str(root)))
        # A relative rule never reaches outside the root; // and ~/ do.
        self.assertFalse(dispatch.path_matches(".env", str(self.base / "outside" / ".env"), str(root)))
        absolute = str(self.base / "outside" / "abs.txt")
        rule = ("//" + absolute[0] + absolute[2:].replace("\\", "/")) if WINDOWS else "//" + absolute.lstrip("/")
        self.assertTrue(dispatch.path_matches(rule, absolute, str(root)))
        self.assertTrue(dispatch.path_matches("~/homefile.txt", str(home / "homefile.txt"), str(root)))

    def decide(self, root, tool, tool_input, start=None):
        with mock.patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": str(self.config)}):
            answer = dispatch.permission({"tool_name": tool, "tool_input": tool_input}, str(root),
                                         str(start or root / "src"))
        return answer["hookSpecificOutput"]["permissionDecision"] if answer else None

    def edition_project(self):
        root = self.project(policy="# Policy\n")
        for relative in (".env", "src/.env", "secrets/key.txt", "composer.json", "src/Invoice.php"):
            (root / relative).parent.mkdir(parents=True, exist_ok=True)
            (root / relative).write_text("x\n", encoding="utf-8")
        return root

    def test_the_editions_rules_decide_below_the_root_as_in_it(self):
        root = self.edition_project()
        read, write = (lambda path: {"file_path": str(path)}), (lambda path: {"file_path": str(path), "content": "x"})
        cases = [
            ("Read", read(root / ".env"), "deny"), ("Read", read(root / "src/.env"), "deny"),
            ("Read", read(root / ".env.local"), "deny"), ("Read", read(root / "secrets/key.txt"), "deny"),
            ("Read", read(root / "src/secrets/key.txt"), "deny"), ("Read", read(root / "composer.json"), "allow"),
            ("Read", read(root / "src/Invoice.php"), "allow"), ("Read", read("/etc/hostname"), None),
            ("Write", write(root / "notes.txt"), "allow"), ("Write", write(root / ".env"), "deny"),
            ("Edit", {"file_path": str(root / ".env"), "old_string": "x", "new_string": "y"}, "deny"),
            ("NotebookEdit", {"notebook_path": str(root / "src/.env"), "new_source": "x"}, "deny"),
            ("Write", write(self.base / "elsewhere.txt"), None),
            ("Read", {"file_path": "../composer.json"}, "allow"),
            ("Bash", {"command": "rm -rf build"}, "deny"), ("Bash", {"command": "sudo -n true"}, "deny"),
            ("Bash", {"command": "chmod 777 notes.txt"}, "deny"), ("Bash", {"command": "echo hi && rm -rf build"}, "deny"),
            ("Bash", {"command": "FOO=1 timeout 5 sudo ls"}, "deny"), ("Bash", {"command": "echo $(sudo id)"}, "deny"),
            # Forms the root refuses too, found by differential runs against the binary.
            ("Bash", {"command": "env rm -rf build"}, "deny"), ("Bash", {"command": "env -i A=1 rm -rf build"}, "deny"),
            ("Bash", {"command": "find . -name x | xargs rm -rf"}, "deny"),
            # A flag after xargs keeps Claude Code from looking through it.
            ("Bash", {"command": "find . -print0 | xargs -0 -I {} rm -rf {}"}, None),
            ("Bash", {"command": "if true; then rm -rf build; fi"}, "deny"),
            ("Bash", {"command": "for d in build; do rm -rf $d; done"}, "deny"),
            ("Bash", {"command": "{ rm -rf build; }"}, "deny"), ("Bash", {"command": "! sudo -n true"}, "deny"),
            # Text an allowed echo prints is not a command: allowed, as in the root.
            ("Bash", {"command": "echo 'rm -rf build'"}, "allow"),
            ("Bash", {"command": "ls"}, "allow"), ("Bash", {"command": "php -v"}, "allow"),
            ("Bash", {"command": "git status --short"}, "allow"),
            ("Bash", {"command": "php -v && ls"}, None), ("Bash", {"command": "git log > out.txt"}, None),
            ("Bash", {"command": "php $(id)"}, None), ("Bash", {"command": "phpunit"}, None),
            ("WebFetch", {"url": "https://github.com/x", "prompt": "p"}, "allow"),
            ("WebFetch", {"url": "https://api.github.com/x", "prompt": "p"}, "allow"),
            ("WebFetch", {"url": "https://example.com/", "prompt": "p"}, None),
            ("WebFetch", {"url": "https://github.com.example.com/", "prompt": "p"}, None),
            ("Agent", {"subagent_type": "Explore", "prompt": "p"}, "deny"),
            ("Agent", {"subagent_type": "php-reviewer", "prompt": "p"}, None),
            ("TodoWrite", {"todos": []}, None),
        ]
        for tool, tool_input, expected in cases:
            with self.subTest(tool=tool, tool_input=tool_input):
                self.assertEqual(self.decide(root, tool, tool_input), expected)

    def test_a_command_rule_reaches_through_what_claude_code_reaches_through(self):
        # Read from Claude Code 2.1.278 and checked by differential runs of
        # the same commands in a root session and one below it.
        root = self.project(policy="# Policy\n")
        settings = root / ".claude" / "settings.json"
        data = json.loads(settings.read_text(encoding="utf-8"))
        data["permissions"] = {"deny": ["Bash(rm -rf:*)", "Bash(sudo:*)"], "allow": []}
        settings.write_text(json.dumps(data), encoding="utf-8")
        refused = ["rm -rf x", "env rm -rf x", "env -u HOME A=1 rm -rf x", "find . | xargs rm -rf",
                   "sudo -u www rm -rf x", "ionice -c3 rm -rf x", "setsid rm -rf x", "timeout -k 5 10s rm -rf x",
                   "nice -n 5 rm -rf x", "command rm -rf x", "A=1 B=2 rm -rf x", "if true; then rm -rf x; fi",
                   "until true; do rm -rf x; done", "{ rm -rf x; }", "! rm -rf x", "echo a && rm -rf x",
                   "echo $(rm -rf x)", "sudo -n true", "FOO=1 timeout 5 sudo ls"]
        let_through = ["xargs -I{} rm -rf {}", "command -v sudo", "command -V sudo", "echo 'rm -rf x'",
                       "bash -c 'rm -rf x'", "/usr/bin/sudo -n true", "rm -Rf x", "/usr/bin/env rm -rf x",
                       "busybox rm -rf x", "coproc rm -rf x"]
        for command in refused:
            with self.subTest(command=command):
                self.assertEqual(self.decide(root, "Bash", {"command": command}), "deny")
        for command in let_through:
            with self.subTest(command=command):
                self.assertIsNone(self.decide(root, "Bash", {"command": command}))

    def test_local_rules_count_and_the_persons_own_are_never_overridden(self):
        root = self.edition_project()
        (root / ".claude" / "settings.local.json").write_text(json.dumps({"permissions": {
            "deny": ["Read(notes/**)"], "ask": ["Bash(git push:*)"]}}), encoding="utf-8")
        (root / "notes").mkdir()
        self.assertEqual(self.decide(root, "Read", {"file_path": str(root / "notes/a.md")}), "deny")
        self.assertEqual(self.decide(root, "Bash", {"command": "git push origin main"}), "ask")
        (self.config / "settings.json").write_text(json.dumps({"permissions": {
            "deny": ["Read(composer.json)"], "ask": ["Bash(php:*)"]}}), encoding="utf-8")
        self.assertIsNone(self.decide(root, "Read", {"file_path": str(root / "composer.json")}))
        self.assertIsNone(self.decide(root, "Bash", {"command": "php -v"}))
        self.assertEqual(self.decide(root, "Read", {"file_path": str(root / "src/Invoice.php")}), "allow")

    def test_a_link_counts_by_where_it_leads(self):
        if not can_link(self.base):
            self.skipTest("this machine makes no symbolic links")
        root = self.edition_project()
        (root / "src" / "out").symlink_to(self.base / "home")
        (root / "src" / "env-link").symlink_to(root / ".env")
        self.assertIsNone(self.decide(root, "Read", {"file_path": str(root / "src/out/x.txt")}))
        self.assertEqual(self.decide(root, "Read", {"file_path": str(root / "src/env-link")}), "deny")

    def test_the_dispatcher_answers_for_the_project_below_the_root(self):
        root = self.edition_project()
        self.setup_dispatcher(root)
        payload = {"tool_name": "Read", "tool_input": {"file_path": str(root / ".env")}}
        done = self.run_dispatcher("permissions", root / "src", json.dumps(payload).encode())
        self.assertEqual(done.returncode, 0, done.stderr)
        answer = json.loads(done.stdout)["hookSpecificOutput"]
        self.assertEqual(answer["permissionDecision"], "deny")
        self.assertIn("Read(.env)", answer["permissionDecisionReason"])
        self.assertEqual(self.run_dispatcher("permissions", root, json.dumps(payload).encode()).stdout, b"",
                         "in the root Claude Code applies the rules itself")
        self.assertEqual(self.run_dispatcher("permissions", root / "src", b"not json").stdout, b"")


class Environment(Fixture):
    def test_the_projects_env_reaches_the_commands_below_the_root(self):
        root = self.project(policy="# Policy\n")
        settings = json.loads((root / ".claude" / "settings.json").read_text(encoding="utf-8"))
        settings["env"] = {"APP_ENV": "local", "FORCE_COLOR": "1", "QUOTED": "it's $HOME", "FLAG": True,
                           "CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH": "1", "ANTHROPIC_MODEL": "x", "bad-key": "x"}
        (root / ".claude" / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
        (root / ".claude" / "settings.local.json").write_text(json.dumps({"env": {"APP_ENV": "testing"}}),
                                                               encoding="utf-8")
        self.assertEqual(dispatch.environment(str(root)), [
            "export APP_ENV=testing", "export FORCE_COLOR=1", "export QUOTED='it'\"'\"'s $HOME'",
            "export FLAG=true"])
        self.setup_dispatcher(root)
        target = self.base / "session.env"
        self.env["CLAUDE_ENV_FILE"] = str(target)
        done = self.run_dispatcher("environment", root / "src")
        self.assertEqual(done.returncode, 0, done.stderr)
        sourced = subprocess.run([BASH, "-c", f"source {shlex.quote(target.as_posix())}; printf '%s|%s' \"$APP_ENV\" \"$QUOTED\""],
                                 capture_output=True, text=True, env=self.env)
        self.assertEqual(sourced.stdout, "testing|it's $HOME")
        target.unlink()
        self.run_dispatcher("environment", root)
        self.assertFalse(target.exists(), "in the root Claude Code sets the env itself")


class Setup(Fixture):
    def test_a_root_session_installs_the_dispatcher_once(self):
        root = self.project(policy="# Policy\n")
        self.setup_dispatcher(root)
        folder = self.config / "ai-accelerator"
        dispatcher = folder / "ai-accelerator-dispatch.sh"
        self.assertEqual(dispatcher.read_bytes(), (SCRIPTS / "claude_dispatch.sh").read_bytes())
        if not WINDOWS:
            self.assertEqual(stat.S_IMODE(dispatcher.stat().st_mode), 0o755)
        self.assertEqual((folder / "ai-accelerator-dispatch.py").read_bytes(),
                         (SCRIPTS / "claude_dispatch.py").read_bytes())
        self.assertEqual((folder / "projects").read_text(encoding="utf-8").splitlines(), [str(root)])
        project = json.loads((root / ".claude" / "settings.json").read_text(encoding="utf-8"))
        expected = []
        for event, groups in project["hooks"].items():
            for group in groups:
                for hook in group["hooks"]:
                    if hook["command"].startswith('"${CLAUDE_PROJECT_DIR}"/.claude/hooks/'):
                        expected.append((event, group.get("matcher", ""), hook["command"].rsplit("/", 1)[1],
                                         hook.get("timeout")))
        expected.append(("PreToolUse", "", "permissions", 10))
        expected.append(("SessionStart", "", "environment", 10))
        expected += [("SessionStart", "", f"policy-{part}", 10) for part in range(1, dispatch.POLICY_PARTS + 1)]
        self.assertEqual(sorted(self.dispatched()), sorted(expected))
        before = (self.config / "settings.json").stat().st_mtime_ns, (self.config / "settings.json").read_bytes()
        time.sleep(0.01)
        self.setup_dispatcher(root)
        after = (self.config / "settings.json").stat().st_mtime_ns, (self.config / "settings.json").read_bytes()
        self.assertEqual(after, before, "nothing is written when all is in place")

    def test_the_persons_own_settings_are_kept_and_projects_share_the_entries(self):
        own = {"model": "opus", "permissions": {"allow": ["Bash(make:*)"]},
               "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify-send done"}]}]}}
        (self.config / "settings.json").write_text(json.dumps(own), encoding="utf-8")
        (self.config / "settings.json").chmod(0o600)
        first, second = self.project("first", policy="# One\n"), self.project("second", policy="# Two\n")
        self.setup_dispatcher(first)
        entries = sorted(self.dispatched())
        self.setup_dispatcher(second)
        settings = self.settings()
        self.assertEqual(sorted(self.dispatched(settings)), entries, "one set of entries for every project")
        self.assertEqual(settings["model"], "opus")
        self.assertEqual(settings["permissions"], own["permissions"])
        self.assertIn({"hooks": [{"type": "command", "command": "notify-send done"}]}, settings["hooks"]["Stop"])
        if not WINDOWS:
            self.assertEqual(stat.S_IMODE((self.config / "settings.json").stat().st_mode), 0o600)
        projects = self.config / "ai-accelerator" / "projects"
        self.assertEqual(projects.read_text(encoding="utf-8").splitlines(), [str(first), str(second)])
        before = projects.stat().st_mtime_ns
        self.setup_dispatcher(first)
        self.assertEqual(projects.stat().st_mtime_ns, before, "a listed root is not written again")
        shutil.rmtree(second)
        self.setup_dispatcher(first)
        self.assertEqual(projects.read_text(encoding="utf-8").splitlines(), [str(first)], "a removed one is dropped")

    def test_projects_starting_together_are_all_listed(self):
        roots = [self.project(f"p{number}", policy="# Policy\n") for number in range(6)]
        payload = json.dumps({"session_id": "s", "transcript_path": str(self.transcript),
                              "hook_event_name": "SessionStart", "source": "startup"}).encode()
        running = [subprocess.Popen([sys.executable, str(root / "memory-bank" / "scripts" / "claude_subfolders.py")],
                                    stdin=subprocess.PIPE, env=self.env, cwd=root) for root in roots]
        for process in running:
            process.stdin.write(payload)
            process.stdin.close()
        for process in running:
            self.assertEqual(process.wait(timeout=60), 0)
        listed = (self.config / "ai-accelerator" / "projects").read_text(encoding="utf-8").splitlines()
        self.assertEqual(sorted(listed), sorted(str(root) for root in roots))
        self.assertEqual(len(self.dispatched()), len(set(self.dispatched())), "no entry twice")

    def test_a_changed_timeout_replaces_its_entry(self):
        root = self.project(policy="# Policy\n")
        self.setup_dispatcher(root)
        settings_file = root / ".claude" / "settings.json"
        project = json.loads(settings_file.read_text(encoding="utf-8"))
        project["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = 12
        settings_file.write_text(json.dumps(project), encoding="utf-8")
        self.setup_dispatcher(root)
        local = [entry for entry in self.dispatched() if entry[2] == "local-context.sh"]
        self.assertEqual(local, [("SessionStart", "", "local-context.sh", 12)])

    def test_nothing_is_written_for_a_hook_another_program_started(self):
        root = self.project(policy="# Policy\n")
        self.setup_dispatcher(root, transcript=self.base / "elsewhere" / "session.jsonl")
        self.assertFalse((self.config / "ai-accelerator").exists())
        self.assertFalse((self.config / "settings.json").exists())

    def test_settings_it_cannot_read_as_an_object_are_left_as_they_are(self):
        root = self.project(policy="# Policy\n")
        for content in ("{not json", "[]", '{"hooks": []}'):
            with self.subTest(content=content):
                (self.config / "settings.json").write_text(content, encoding="utf-8")
                self.setup_dispatcher(root)
                self.assertEqual((self.config / "settings.json").read_text(encoding="utf-8"), content)

    def test_a_file_named_off_turns_it_off_for_good(self):
        root = self.project(hooks={"echo.sh": Dispatcher.ECHO}, policy="# Policy\n")
        self.setup_dispatcher(root)
        self.assertTrue(self.run_dispatcher("echo.sh", root / "src").stdout)
        (self.config / "ai-accelerator" / "off").write_text("", encoding="utf-8")
        for hook in ("echo.sh", "policy-1"):
            self.assertEqual(self.run_dispatcher(hook, root / "src").stdout, b"")
        # The person took the entries out: a session in the root puts none back.
        (self.config / "settings.json").write_text("{}", encoding="utf-8")
        self.setup_dispatcher(root)
        self.assertEqual(self.settings(), {})

    def test_a_linked_settings_file_is_written_where_it_lives(self):
        if not can_link(self.base):
            self.skipTest("this machine makes no symbolic links")
        dotfiles = self.base / "dotfiles"
        dotfiles.mkdir()
        (dotfiles / "settings.json").write_text('{"model": "opus"}', encoding="utf-8")
        (self.config / "settings.json").symlink_to(dotfiles / "settings.json")
        root = self.project(policy="# Policy\n")
        self.setup_dispatcher(root)
        self.assertTrue((self.config / "settings.json").is_symlink())
        written = json.loads((dotfiles / "settings.json").read_text(encoding="utf-8"))
        self.assertEqual(written["model"], "opus")
        self.assertTrue(self.dispatched(written))

    def test_a_folder_that_is_a_link_gets_nothing(self):
        if not can_link(self.base):
            self.skipTest("this machine makes no symbolic links")
        root = self.project(policy="# Policy\n")
        elsewhere = self.base / "elsewhere"
        elsewhere.mkdir()
        (self.config / "ai-accelerator").symlink_to(elsewhere)
        self.setup_dispatcher(root)
        self.assertEqual(list(elsewhere.iterdir()), [])
        self.assertFalse((self.config / "settings.json").exists())


class Dispatcher(Fixture):
    ECHO = 'printf "ran in %s for %s: %s" "$PWD" "$CLAUDE_PROJECT_DIR" "$(cat)"\n'

    def test_a_session_below_the_root_runs_the_projects_hook_there(self):
        root = self.project(hooks={"echo.sh": self.ECHO}, policy="# Policy\nRule.\n")
        self.setup_dispatcher(root)
        done = self.run_dispatcher("echo.sh", root / "src" / "Billing", b'{"hook": 1}')
        self.assert_ran(done, root, '{"hook": 1}')

    def test_nothing_runs_in_the_root_an_unregistered_project_or_another_checkout(self):
        root = self.project(hooks={"echo.sh": self.ECHO}, policy="# Policy\n")
        self.setup_dispatcher(root)
        other = self.project("other", hooks={"echo.sh": self.ECHO}, policy="# Other\n")
        nested = root / "src" / "vendored"
        nested.mkdir()
        git(nested, "init", "-q")
        for hook, start in (("echo.sh", root), ("echo.sh", other / "src"), ("echo.sh", nested),
                            ("../echo.sh", root / "src"), (".echo.sh", root / "src"), ("", root / "src"),
                            ("missing.sh", root / "src"), ("policy-1", root), ("policy-1", nested)):
            with self.subTest(hook=hook, start=start):
                done = self.run_dispatcher(hook, start)
                self.assertEqual((done.returncode, done.stdout), (0, b""), done.stderr)

    def test_a_project_installed_in_a_folder_of_a_larger_checkout(self):
        monorepo = self.base / "monorepo"
        monorepo.mkdir()
        git(monorepo, "init", "-q")
        root = self.project("monorepo/backend", hooks={"echo.sh": self.ECHO}, policy="# Policy\n")
        shutil.rmtree(root / ".git")
        self.setup_dispatcher(root)
        self.assert_ran(self.run_dispatcher("echo.sh", root / "src"), root)

    def test_the_nearest_installed_project_is_the_sessions(self):
        outer = self.project("outer", hooks={"echo.sh": self.ECHO}, policy="# Outer\n")
        self.setup_dispatcher(outer)
        inner = outer / "src" / "inner"
        (inner / "memory-bank" / "scripts").mkdir(parents=True)
        (inner / "memory-bank" / "scripts" / "context.py").write_text("# another runtime\n", encoding="utf-8")
        (inner / "lib").mkdir()
        self.assertEqual(self.run_dispatcher("echo.sh", inner / "lib").stdout, b"", "not listed: not served")
        self.assertTrue(self.run_dispatcher("echo.sh", outer / "src").stdout)
        listed = self.project("outer/src/listed", hooks={"echo.sh": self.ECHO}, policy="# Listed\n")
        shutil.rmtree(listed / ".git")
        self.setup_dispatcher(listed)
        self.assert_ran(self.run_dispatcher("echo.sh", listed / "src"), listed)

    def test_a_folder_outside_git_follows_the_registered_root(self):
        if inside_git(self.base):
            self.skipTest("the temporary folder is inside a Git checkout")
        root = self.project(hooks={"echo.sh": self.ECHO}, policy="# Policy\n")
        shutil.rmtree(root / ".git")
        self.setup_dispatcher(root)
        self.assert_ran(self.run_dispatcher("echo.sh", root / "src"), root)

    def test_the_policy_comes_in_parts(self):
        text = "# Policy\n" + "".join(f"## Rule {number}\n" + "Keep it.\n" * 400 for number in range(3))
        root = self.project(policy=text)
        self.setup_dispatcher(root)
        outputs = [self.run_dispatcher(f"policy-{part}", root / "src").stdout.decode("utf-8")
                   for part in range(1, dispatch.POLICY_PARTS + 1)]
        given = [output for output in outputs if output]
        self.assertGreater(len(given), 1)
        self.assertEqual("".join(output.split(":\n\n", 1)[1] for output in given), text)
        for output in given:
            self.assertLessEqual(dispatch.width(output.strip()), dispatch.INLINE)
        self.assertEqual(self.run_dispatcher("policy-x", root / "src").stdout, b"")

    def test_the_policy_is_utf8_whatever_the_locale(self):
        root = self.project(policy="# Policy \u2014 rules\nKeep \u00e9 and \u2192.\n")
        self.setup_dispatcher(root)
        self.env.update(PYTHONIOENCODING="ascii", LC_ALL="C")
        done = self.run_dispatcher("policy-1", root / "src")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(done.stdout.decode("utf-8").endswith("# Policy \u2014 rules\nKeep \u00e9 and \u2192.\n"))

    @unittest.skipIf(WINDOWS and not any(shutil.which(name) for name in ("python3", "jq", "php")),
                     "bash-validator.sh reads its input with python3, jq or php")
    def test_the_guard_still_blocks_and_the_memory_cli_is_let_through(self):
        root = self.project(policy="# Policy\n")
        shutil.copy2(ROOT / "PHP Core" / ".claude" / "hooks" / "bash-validator.sh",
                     root / ".claude" / "hooks" / "bash-validator.sh")
        self.setup_dispatcher(root)
        start = root / "src"

        def call(command):
            payload = {"session_id": "guard", "hook_event_name": "PreToolUse", "tool_name": "Bash",
                       "tool_input": {"command": command}, "cwd": str(start)}
            return self.run_dispatcher("bash-validator.sh", start, json.dumps(payload).encode())

        # As the runtime writes the step: the root through shlex.quote, so a
        # Windows path keeps its backslashes.
        quoted = shlex.quote(str(root))
        save = (f"cd {quoted} && echo -E '{{\"progress\": \"The rollback runs DROP TABLE invoices.\"}}' | "
                f"python3 memory-bank/scripts/context.py --root {quoted} native-result --request r --token t "
                "--input - --json")
        done = call(save)
        self.assertEqual(done.returncode, 0, done.stderr)
        decision = json.loads(done.stdout)["hookSpecificOutput"]
        self.assertEqual((decision["hookEventName"], decision["permissionDecision"]), ("PreToolUse", "allow"))
        done = call("git status")
        self.assertEqual((done.returncode, done.stdout), (0, b""))
        done = call("rm -rf /")
        self.assertEqual(done.returncode, 2)
        self.assertNotIn(b"allow", done.stdout)


class Worktrees(Fixture):
    """A linked worktree has none of an installation Git does not track, so
    Claude Code started there ran nothing of it. The dispatcher runs the
    root's hooks there for the worktree: the root keeps the tooling and the
    memory, the worktree is the project its branch names the task of."""
    ECHO = ('printf "%s|%s|%s|%s|%s|%s" "$PWD" "$CLAUDE_PROJECT_DIR" "$ACCELERATOR_HOME" '
            '"$ACCELERATOR_STATE_DIR" "$ACCELERATOR_PROJECT_DIR" "$(cat)"\n')

    def committed(self):
        """A project whose code is committed and whose installation is not."""
        root = self.project(hooks={"echo.sh": self.ECHO}, policy="# Policy\nRule.\n")
        (root / "src" / "Billing" / "Invoice.php").write_text("<?php\n", encoding="utf-8")
        git(root, "add", "src")
        git(root, "commit", "-qm", "base")
        self.setup_dispatcher(root)
        return root

    def assert_served(self, done, root, project, payload="{}"):
        self.assertEqual(done.returncode, 0, done.stderr)
        fields = done.stdout.decode().split("|", 5)
        self.assertEqual(len(fields), 6, done.stdout)
        where, named, home, state, served, given = fields
        self.assertEqual([shown(where), shown(named), shown(served), given], [shown(project)] * 3 + [payload],
                         done.stdout)
        self.assertEqual((home, state), (str(root), str(root)))

    def test_a_worktree_in_the_project_or_beside_it_gets_the_roots_hooks(self):
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "refunds"
        beside = self.base / "project-credit-notes"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        git(root, "worktree", "add", "-q", str(beside), "-b", "feature/SHOP-10-credit-notes")
        for project in (inner, beside):
            for start in (project, project / "src" / "Billing"):
                with self.subTest(start=start):
                    self.assert_served(self.run_dispatcher("echo.sh", start, b'{"hook": 1}'), root, project,
                                       '{"hook": 1}')
        self.assertFalse((inner / "memory-bank").exists() or (beside / "memory-bank").exists(),
                         "nothing is written into a worktree")

    def test_a_project_in_a_folder_of_a_monorepo_is_that_folder_of_its_worktree(self):
        monorepo = self.base / "monorepo"
        monorepo.mkdir()
        git(monorepo, "init", "-q")
        root = self.project("monorepo/backend", hooks={"echo.sh": self.ECHO}, policy="# Policy\n")
        shutil.rmtree(root / ".git")
        (root / "src" / "Billing" / "Invoice.php").write_text("<?php\n", encoding="utf-8")
        git(monorepo, "add", "backend/src")
        git(monorepo, "commit", "-qm", "base")
        self.setup_dispatcher(root)
        worktree = self.base / "monorepo-feature"
        git(monorepo, "worktree", "add", "-q", str(worktree), "-b", "feature/x")
        self.assert_served(self.run_dispatcher("echo.sh", worktree / "backend" / "src"), root, worktree / "backend")
        done = self.run_dispatcher("echo.sh", worktree)
        self.assertEqual((done.returncode, done.stdout), (0, b""), "the monorepo's root is not the project")

    def test_a_worktree_with_an_installation_of_its_own_or_of_another_repository_is_left_alone(self):
        root = self.committed()
        own = root / ".claude" / "worktrees" / "own"
        git(root, "worktree", "add", "-q", str(own), "-b", "feature/own")
        (own / "memory-bank" / "scripts").mkdir(parents=True)
        (own / "memory-bank" / "scripts" / "context.py").write_text("# its own runtime\n", encoding="utf-8")
        other = self.base / "other"
        (other / "src").mkdir(parents=True)
        (other / "src" / "a.php").write_text("<?php\n", encoding="utf-8")
        git(other, "init", "-q")
        git(other, "add", ".")
        git(other, "commit", "-qm", "base")
        foreign = self.base / "other-feature"
        git(other, "worktree", "add", "-q", str(foreign), "-b", "feature/x")
        # A .git file naming a worktree Git does not know, and a submodule's.
        forged = self.base / "forged"
        forged.mkdir()
        (forged / ".git").write_text(f"gitdir: {root / '.git' / 'worktrees' / 'ghost'}\n", encoding="utf-8")
        module = root / "src" / "module"
        module.mkdir()
        (module / ".git").write_text("gitdir: ../../.git/modules/module\n", encoding="utf-8")
        for start in (own, own / "src" / "Billing", foreign, foreign / "src", forged, module):
            with self.subTest(start=start):
                done = self.run_dispatcher("echo.sh", start)
                self.assertEqual((done.returncode, done.stdout), (0, b""), done.stderr)

    def test_the_policy_comes_whatever_claude_code_was_told_about_imports(self):
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "refunds"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        # The dialog's Yes is filed under the main checkout, and the worktree
        # has no .claude/CLAUDE.md to import anything from.
        (self.config / ".claude.json").write_text(json.dumps({"projects": {key_of(root): {
            "hasClaudeMdExternalIncludesApproved": True}}}), encoding="utf-8")
        done = self.run_dispatcher("policy-1", inner)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(done.stdout.decode("utf-8").endswith("# Policy\nRule.\n"), done.stdout)
        self.assertEqual(self.run_dispatcher("policy-1", root / "src").stdout, b"", "below the root it is imported")
        (inner / "AGENTS.md").write_text("# The worktree's own\n", encoding="utf-8")
        self.assertEqual(self.run_dispatcher("policy-1", inner).stdout, b"")

    def test_the_sessions_commands_get_the_variables_the_save_step_needs(self):
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "refunds"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        target = self.base / "session.env"
        self.env["CLAUDE_ENV_FILE"] = str(target)
        done = self.run_dispatcher("environment", inner / "src")
        self.assertEqual(done.returncode, 0, done.stderr)
        sourced = subprocess.run(
            [BASH, "-c", f"source {shlex.quote(target.as_posix())}; "
                         'printf "%s|%s|%s" "$ACCELERATOR_HOME" "$ACCELERATOR_STATE_DIR" "$ACCELERATOR_PROJECT_DIR"'],
            capture_output=True, text=True, env=self.env)
        home, state, project = sourced.stdout.split("|")
        self.assertEqual((home, state, shown(project)), (str(root), str(root), shown(inner)))
        target.unlink()
        self.run_dispatcher("environment", root / "src")
        self.assertNotIn("ACCELERATOR_", target.read_text(encoding="utf-8") if target.exists() else "",
                         "below the root the root itself is the project")

    def test_the_roots_rules_decide_for_the_worktrees_files_and_keep_the_roots_secrets(self):
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "refunds"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        for folder in (root, inner):
            (folder / ".env").write_text("SECRET=x\n", encoding="utf-8")
            (folder / "secrets").mkdir(exist_ok=True)
            (folder / "secrets" / "key.txt").write_text("x\n", encoding="utf-8")

        def decide(path, tool="Read"):
            with mock.patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": str(self.config)}):
                answer = dispatch.permission({"tool_name": tool, "tool_input": {"file_path": str(path)}},
                                             str(root), str(inner / "src"), str(inner))
            return answer["hookSpecificOutput"]["permissionDecision"] if answer else None

        self.assertEqual([decide(inner / ".env"), decide(inner / "secrets" / "key.txt"),
                          decide(inner / "src" / "Billing" / "Invoice.php")], ["deny", "deny", "allow"])
        self.assertEqual(decide(inner / ".env", "Edit"), "deny")
        # The root's own secrets stay denied; its other files are the
        # worktree session's to ask about.
        self.assertEqual([decide(root / ".env"), decide(root / "secrets" / "key.txt"),
                          decide(root / "src" / "Billing" / "Invoice.php")], ["deny", "deny", None])
        payload = {"tool_name": "Read", "tool_input": {"file_path": str(inner / ".env")}}
        done = self.run_dispatcher("permissions", inner / "src", json.dumps(payload).encode())
        self.assertEqual(json.loads(done.stdout)["hookSpecificOutput"]["permissionDecision"], "deny")


class InstallTime(Fixture):
    """A person installing in a terminal gets the dispatcher at once; a
    scripted install leaves it to the project's first session in its root."""

    def install(self, target, terminal):
        command = [sys.executable, str(ROOT / "scripts" / "install_accelerator.py"), "--edition", "PHP Core",
                   "--target", str(target)]
        if not terminal:
            done = subprocess.run(command, env=self.env, cwd=ROOT, capture_output=True, text=True,
                                  stdin=subprocess.DEVNULL, timeout=300)
            return done.returncode, done.stdout + done.stderr
        import pty
        leader, follower = pty.openpty()
        # "stderr-and-stdin": stdout goes elsewhere, as `| tee` sends it.
        piped = open(self.base / "stdout.txt", "wb") if terminal == "stderr-and-stdin" else None
        process = subprocess.Popen(command, stdin=follower, stdout=piped or follower, stderr=follower, env=self.env,
                                   cwd=ROOT)
        os.close(follower)
        output = b""
        while True:
            try:
                chunk = os.read(leader, 65536)
            except OSError:
                break
            if not chunk:
                break
            output += chunk
        os.close(leader)
        code = process.wait(timeout=300)
        if piped:
            piped.close()
            output += (self.base / "stdout.txt").read_bytes()
        return code, output.decode("utf-8", "replace")

    def target(self):
        target = self.base / "shop"
        target.mkdir()
        git(target, "init", "-q")
        return target

    @unittest.skipIf(WINDOWS, "a pseudo-terminal is POSIX's")
    def test_a_person_installing_in_a_terminal_gets_the_dispatcher_at_once(self):
        target = self.target()
        code, output = self.install(target, terminal=True)
        self.assertEqual(code, 0, output[-2000:])
        self.assertIn("CLAUDE\tbelow-root\t" + str(self.config / "ai-accelerator"), output)
        listed = (self.config / "ai-accelerator" / "projects").read_text(encoding="utf-8").splitlines()
        self.assertEqual(listed, [str(target.resolve())])
        self.assertIn(("PreToolUse", "", "permissions", 10), self.dispatched())

    def test_a_scripted_install_leaves_it_to_the_first_session_in_the_root(self):
        target = self.target()
        code, output = self.install(target, terminal=False)
        self.assertEqual(code, 0, output[-2000:])
        self.assertNotIn("below-root", output)
        self.assertFalse((self.config / "ai-accelerator").exists())
        self.assertFalse((self.config / "settings.json").exists())

    def test_a_call_into_the_module_never_writes_outside_the_project(self):
        spec = importlib.util.spec_from_file_location("installer_under_test", ROOT / "scripts" / "install_accelerator.py")
        installer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installer)
        target = self.target()
        with open(os.devnull, "w") as quiet, mock.patch.dict(os.environ, {"CLAUDE_CONFIG_DIR": str(self.config)}), \
                mock.patch.object(sys.stdin, "isatty", return_value=True), \
                mock.patch.object(sys.stderr, "isatty", return_value=True), mock.patch("sys.stdout", new=quiet):
            self.assertEqual(installer.install(ROOT, "PHP Core", target, ["claude"], False, False, False), 0)
        self.assertFalse((self.config / "ai-accelerator").exists())

    @unittest.skipIf(WINDOWS, "a pseudo-terminal is POSIX's")
    def test_output_through_tee_still_counts_as_a_terminal(self):
        target = self.target()
        code, output = self.install(target, terminal="stderr-and-stdin")
        self.assertEqual(code, 0, output[-2000:])
        self.assertIn("CLAUDE\tbelow-root", output)
        self.assertTrue((self.config / "ai-accelerator" / "projects").is_file())

    @unittest.skipIf(WINDOWS, "a pseudo-terminal is POSIX's")
    def test_nothing_without_a_claude_code_configuration_folder(self):
        shutil.rmtree(self.config)
        target = self.target()
        code, output = self.install(target, terminal=True)
        self.assertEqual(code, 0, output[-2000:])
        self.assertNotIn("below-root", output)
        self.assertFalse(self.config.exists())


class RootSession(Fixture):
    def test_the_root_sessions_start_hook_sets_it_up_and_a_folder_below_gets_the_banner(self):
        root = self.base / "shop"
        for relative in (".claude", "memory-bank/scripts", "memory-bank/templates", "project-brain/config",
                         "project-brain/schemas", "project-brain/templates"):
            shutil.copytree(ROOT / "PHP Core" / relative, root / relative,
                            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        for name in ("AGENTS.md", "VERSION"):
            shutil.copy2(ROOT / "PHP Core" / name, root / name)
        (root / "src").mkdir()
        git(root, "init", "-q")
        payload = json.dumps({"session_id": "root", "transcript_path": str(self.transcript),
                              "hook_event_name": "SessionStart", "source": "startup"}).encode()
        env = dict(self.env, CLAUDE_PROJECT_DIR=str(root))
        done = subprocess.run([BASH, (root / ".claude" / "hooks" / "local-context.sh").as_posix()], input=payload,
                              env=env, cwd=root, capture_output=True, timeout=60)
        self.assertEqual(done.returncode, 0, done.stderr)
        settings = self.config / "settings.json"
        deadline = time.time() + 30
        while time.time() < deadline and not (settings.exists() and self.dispatched()):
            time.sleep(0.2)
        self.assertIn(("SessionStart", "", "local-context.sh", 10), self.dispatched())
        # Without a transcript the hook's own setup, in the background again,
        # writes nothing while the test's folder is removed.
        done = self.run_dispatcher("local-context.sh", root / "src", b'{"hook_event_name": "SessionStart"}')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn(b"Project Context", done.stdout)
        policy = self.run_dispatcher("policy-1", root / "src").stdout.decode("utf-8")
        self.assertTrue(policy.startswith(f"Contents of {root / 'AGENTS.md'} (the project's policy), part 1 of "))


if __name__ == "__main__":
    unittest.main()
