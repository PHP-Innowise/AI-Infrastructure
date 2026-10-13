#!/usr/bin/env python3
"""Executable bits the repository must record in its Git index.

The rule is deliberately small. A tracked file must be 100755 when it is

1. a `*.sh` file inside a `hooks/` directory - every edition, every tool tree
   (`.claude/`, `.cursor/`, `.codex/`), Infrastructure-Creator, and any
   generator asset;
2. a hook script a tracked wiring file runs - every `<tool>/hooks/<name>.sh`
   a `command` in `.claude/settings.json`, `.cursor/hooks.json` or
   `.codex/hooks.json` names, whatever form wraps it (a bare relative path, a
   path anchored on `${CLAUDE_PROJECT_DIR}`, a launcher that finds and execs
   the script named as its argument), resolved against the directory holding
   the tool tree; it must
   also be tracked (an inline shell snippet, such as the Notification hook's
   `case ... esac`, names no script and is skipped); or
3. a file at the repository root that starts with a `#!` shebang - the
   launchers people run as `./collect`, `./kit3`, `./harness-server`,
   `./accelerator-app`.

Any other script is invoked through an interpreter (`python3 x.py`,
`bash x.sh`) and keeps whatever bit it has. The index, not the disk, is read
because it is what ships: a checkout without filesystem modes (Windows,
`core.fileMode=false`) shows nothing on disk, and a bit set only on disk is
lost at the next commit.

Regression: the Cursor and Codex copies of subagent-dispatch.sh were 100644 in
all four editions while their .claude canon was 100755. Cursor wires that hook
as a direct command (subagentStop), so every installed project got exit status
126 and the write-agent lock taken by subagent-gate was never released.

Fix a failure with `git update-index --chmod=+x -- <path>` (plus `chmod +x` on
disk), or for a generated mirror `python3 scripts/build_mirrors.py --write`.

Run: python3 -m unittest tests.test_file_modes
"""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent
WIRING = {
    ".claude": "settings.json",
    ".cursor": "hooks.json",
    ".codex": "hooks.json",
}
sys.path.insert(0, str(ROOT / "scripts"))
from check_routes import wired_hook_scripts as scripts_named  # noqa: E402
from file_modes import FileModes  # noqa: E402


def tracked_modes() -> dict[str, str] | None:
    """Repository-relative path -> index mode, or None outside a checkout."""
    try:
        result = subprocess.run(
            ["git", "ls-files", "--stage", "-z", "--full-name"],
            cwd=str(ROOT),
            capture_output=True,
        )
    except OSError:
        return None
    if result.returncode != 0:
        return None
    modes: dict[str, str] = {}
    for record in result.stdout.split(b"\0"):
        meta, tab, raw = record.partition(b"\t")
        fields = meta.split()
        if tab and len(fields) == 3:
            modes[raw.decode("utf-8", "surrogateescape")] = fields[0].decode("ascii")
    return modes


def is_hook_script(path: str) -> bool:
    pure = PurePosixPath(path)
    return pure.suffix == ".sh" and "hooks" in pure.parts[:-1]


def wiring_files(modes: dict[str, str]) -> list[str]:
    found = []
    for path in modes:
        parts = PurePosixPath(path).parts
        if len(parts) >= 2 and WIRING.get(parts[-2]) == parts[-1]:
            found.append(path)
    return sorted(found)


def wired_scripts(config: object) -> list[str]:
    """Every tool-tree-relative hook script a wiring file's commands name."""
    scripts: list[str] = []

    def walk(node: object) -> None:
        if isinstance(node, dict):
            for key, value in node.items():
                if key == "command" and isinstance(value, str):
                    scripts.extend(scripts_named(value))
                else:
                    walk(value)
        elif isinstance(node, list):
            for item in node:
                walk(item)

    if isinstance(config, dict):
        walk(config.get("hooks", {}))
    return scripts


class FileModeTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        modes = tracked_modes()
        if modes is None:
            raise unittest.SkipTest("not a Git checkout; the index is the authority")
        cls.modes = modes

    def assert_executable(self, paths: list[str], rule: str) -> None:
        self.assertTrue(paths, f"rule {rule} matched nothing; the layout moved")
        wrong = [path for path in paths if self.modes.get(path) != "100755"]
        self.assertEqual(
            wrong,
            [],
            f"rule {rule}: not 100755 in the Git index; fix with "
            "`git update-index --chmod=+x -- <path>` (and `chmod +x`), or "
            "`python3 scripts/build_mirrors.py --write` for a mirror",
        )

    def test_hook_scripts_are_executable(self) -> None:
        hooks = sorted(path for path in self.modes if is_hook_script(path))
        # Every edition and tool tree contributes; a short list means the
        # scan, not the repository, is wrong.
        for tree in (".claude/hooks/", ".cursor/hooks/", ".codex/hooks/"):
            self.assertTrue(any(tree in path for path in hooks), tree)
        self.assertTrue(any(path.startswith("Infrastructure-Creator/") for path in hooks))
        self.assert_executable(hooks, "1 (hooks/*.sh)")

    def test_wired_hook_scripts_are_tracked_and_executable(self) -> None:
        scripts: list[str] = []
        for wiring in wiring_files(self.modes):
            base = PurePosixPath(wiring).parent.parent
            config = json.loads((ROOT / wiring).read_text(encoding="utf-8"))
            named = wired_scripts(config)
            self.assertTrue(named, f"{wiring} names no hook script")
            for relative in named:
                script = (base / relative).as_posix()
                with self.subTest(wiring=wiring, script=relative):
                    self.assertIn(script, self.modes, "wired script is not tracked")
                scripts.append(script)
        self.assert_executable(sorted(set(scripts)), "2 (wired hook scripts)")

    def test_root_launchers_are_executable(self) -> None:
        launchers = []
        for path in self.modes:
            if "/" in path or not (ROOT / path).is_file():
                continue
            with (ROOT / path).open("rb") as handle:
                if handle.read(2) == b"#!":
                    launchers.append(path)
        self.assertIn("collect", launchers)
        self.assert_executable(sorted(launchers), "3 (root launchers)")


@unittest.skipIf(os.name == "nt", "the filesystem has no executable bit")
@unittest.skipUnless(shutil.which("git"), "Git is required for an index")
class IntentToAddTest(unittest.TestCase):
    """`scripts/file_modes.py` leaves an intent-to-add entry alone.

    Regression: `git add -N` records the empty blob with the intent-to-add
    flag. FileModes read it as an ordinary entry, and repairing its mode
    through `update-index --index-info` staged that empty blob and dropped
    the flag, so a plain `git commit` recorded a 0-byte mirror.
    """

    def git(self, *args: str) -> str:
        result = subprocess.run(
            ["git", *args], cwd=str(self.repo), capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="file-modes-ita-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        self.git("-c", "init.defaultBranch=main", "init", "-q")
        # One directory down, as "PHP Core" and "Cms/wordpress" sit.
        self.base = self.repo / "Synthetic Edition"
        self.path = self.base / "hooks" / "new-hook.sh"
        self.path.parent.mkdir(parents=True)
        self.path.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
        self.path.chmod(0o644)
        self.git("add", "-N", "--", "Synthetic Edition/hooks/new-hook.sh")

    def status(self) -> str:
        return self.git("status", "--porcelain=v1", "-z", "--", "Synthetic Edition")

    def test_setting_the_bit_keeps_the_entry_intent_to_add(self) -> None:
        self.assertEqual(self.status(), " A Synthetic Edition/hooks/new-hook.sh\0")
        modes = FileModes(self.base)
        # Untracked for mode purposes: the disk decides.
        self.assertIs(modes.is_executable(self.path), False)
        self.assertTrue(modes.set_executable(self.path, True))
        modes.flush()
        self.assertTrue(self.path.stat().st_mode & stat.S_IXUSR)
        # Before the fix: "AM", an empty 100755 blob staged in its place.
        self.assertEqual(self.status(), " A Synthetic Edition/hooks/new-hook.sh\0")

    def test_untrusted_filesystem_gives_no_verdict_and_writes_nothing(self) -> None:
        self.git("config", "core.fileMode", "false")
        modes = FileModes(self.base)
        self.assertIsNone(modes.is_executable(self.path))
        self.assertFalse(modes.set_executable(self.path, True))
        modes.flush()
        self.assertEqual(self.status(), " A Synthetic Edition/hooks/new-hook.sh\0")



class LineEndingTest(unittest.TestCase):
    """Tracked text keeps LF in the index, so every checkout gets LF.

    bash fails on a carriage return in a hook, and the byte-comparing mirror,
    parity and policy-lock gates reported hundreds of false drifts in a CRLF
    clone. Client-supplied task material under */Task/ is exempt
    (`.gitattributes`: `*/Task/** -text`).
    """

    def test_no_tracked_text_file_carries_crlf_in_the_index(self) -> None:
        result = subprocess.run(
            ["git", "ls-files", "--eol", "-z"],
            cwd=str(ROOT), capture_output=True, check=True,
        )
        offenders = []
        for record in result.stdout.split(b"\0"):
            if not record:
                continue
            info, _, path = record.decode("utf-8", "replace").partition("\t")
            index_eol = info.split()[0]
            if "crlf" in index_eol and "/Task/" not in f"/{path}":
                offenders.append(path)
        self.assertEqual([], offenders)

    def test_root_attributes_pin_lf(self) -> None:
        attributes = (ROOT / ".gitattributes").read_text(encoding="utf-8")
        self.assertIn("* text=auto eol=lf\n", attributes)

if __name__ == "__main__":
    unittest.main()
