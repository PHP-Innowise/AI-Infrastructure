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

ROOT = Path(__file__).resolve().parents[1]
EDITIONS = ("PHP Core", "Laravel", "Symfony", "Cms/wordpress")
SCRIPTS = ROOT / "PHP Core" / "memory-bank" / "scripts"


def load_dispatch():
    spec = importlib.util.spec_from_file_location("claude_dispatch_under_test", SCRIPTS / "claude_dispatch.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


dispatch = load_dispatch()


def inside_git(folder):
    """Whether a folder above `folder` holds a .git (a machine's /tmp can)."""
    return any((parent / ".git").exists() for parent in Path(folder).parents)


def git(folder, *args):
    subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                    "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
                   cwd=folder, check=True, capture_output=True)


@unittest.skipIf(os.name == "nt", "the dispatcher is not set up on Windows")
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
        marker = shlex.quote(str(self.config / "ai-accelerator" / "ai-accelerator-dispatch.sh")) + " "
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
        return subprocess.run(["bash", str(self.config / "ai-accelerator" / "ai-accelerator-dispatch.sh"), hook],
                              input=payload, env=env, cwd=start, capture_output=True)


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
        (self.config / ".claude.json").write_text(json.dumps({"projects": {str(folder): {
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


class ProjectKey(Fixture):
    """Where Claude Code 2.1 files a session's project settings."""

    def test_the_checkout_outside_git_the_start_folder_and_a_linked_tree_its_main_checkout(self):
        root = self.project(policy="# Policy\n")
        self.assertEqual(dispatch.project_key(str(root / "src" / "Billing")), str(root))
        self.assertEqual(dispatch.project_key(str(root)), str(root))
        plain = self.base / "plain" / "src"
        plain.mkdir(parents=True)
        if not inside_git(self.base):
            self.assertEqual(dispatch.project_key(str(plain)), str(plain))
        git(root, "add", ".")
        git(root, "commit", "-qm", "base")
        linked = self.base / "linked"
        git(root, "worktree", "add", "-q", str(linked))
        (linked / "src").mkdir(exist_ok=True)
        self.assertEqual(dispatch.project_key(str(linked / "src")), str(root))
        # A .git file that names no linked tree of a repository counts as is.
        forged = self.base / "forged"
        (forged / "src").mkdir(parents=True)
        (forged / ".git").write_text(f"gitdir: {root / '.git'}\n", encoding="utf-8")
        self.assertEqual(dispatch.project_key(str(forged / "src")), str(forged))


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


class Setup(Fixture):
    def test_a_root_session_installs_the_dispatcher_once(self):
        root = self.project(policy="# Policy\n")
        self.setup_dispatcher(root)
        folder = self.config / "ai-accelerator"
        dispatcher = folder / "ai-accelerator-dispatch.sh"
        self.assertEqual(dispatcher.read_bytes(), (SCRIPTS / "claude_dispatch.sh").read_bytes())
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
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.decode(), f'ran in {root} for {root}: {{"hook": 1}}')

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
        done = self.run_dispatcher("echo.sh", root / "src")
        self.assertTrue(done.stdout.decode().startswith(f"ran in {root} for {root}"), done.stdout)

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
        done = self.run_dispatcher("echo.sh", listed / "src")
        self.assertTrue(done.stdout.decode().startswith(f"ran in {listed} for {listed}"), done.stdout)

    def test_a_folder_outside_git_follows_the_registered_root(self):
        if inside_git(self.base):
            self.skipTest("the temporary folder is inside a Git checkout")
        root = self.project(hooks={"echo.sh": self.ECHO}, policy="# Policy\n")
        shutil.rmtree(root / ".git")
        self.setup_dispatcher(root)
        done = self.run_dispatcher("echo.sh", root / "src")
        self.assertTrue(done.stdout.decode().startswith(f"ran in {root} for {root}"), done.stdout)

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

        save = (f"cd {root} && echo -E '{{\"progress\": \"The rollback runs DROP TABLE invoices.\"}}' | "
                f"python3 memory-bank/scripts/context.py --root {root} native-result --request r --token t "
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


@unittest.skipIf(os.name == "nt", "the dispatcher is not set up on Windows")
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
        done = subprocess.run(["bash", str(root / ".claude" / "hooks" / "local-context.sh")], input=payload,
                              env=env, cwd=root, capture_output=True, timeout=60)
        self.assertEqual(done.returncode, 0, done.stderr)
        settings = self.config / "settings.json"
        deadline = time.time() + 30
        while time.time() < deadline and not (settings.exists() and self.dispatched()):
            time.sleep(0.2)
        self.assertIn(("SessionStart", "", "local-context.sh", 10), self.dispatched())
        done = self.run_dispatcher("local-context.sh", root / "src", payload)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn(b"Project Context", done.stdout)
        policy = self.run_dispatcher("policy-1", root / "src").stdout.decode("utf-8")
        self.assertTrue(policy.startswith(f"Contents of {root / 'AGENTS.md'} (the project's policy), part 1 of "))


if __name__ == "__main__":
    unittest.main()
