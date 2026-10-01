"""Offline Windows command-boundary checks; no cmd.exe or provider launch."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness" / "src"))
from harness import windows_commands


class WindowsCommandTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.node = self.root / "node.exe"
        self.node.write_bytes(b"fixture")

    def npm_shim(self, command="codex", package="@openai/codex", bin_name="codex",
                 target="bin/codex.js"):
        shim = self.root / (command + ".cmd")
        shim.write_text("untrusted batch text", encoding="utf-8")
        folder = self.root / "node_modules" / package
        folder.mkdir(parents=True, exist_ok=True)
        script = folder / target
        script.parent.mkdir(parents=True, exist_ok=True)
        script.write_text("console.log('fixture')", encoding="utf-8")
        (folder / "package.json").write_text(json.dumps({"name": package, "bin": {bin_name: target}}), encoding="utf-8")
        return shim, script

    def windows(self):
        return patch.multiple(windows_commands, WINDOWS=True), patch.object(
            windows_commands.shutil, "which", side_effect=lambda value: str(self.node) if value in ("node.exe", "node") else None)

    def test_allowlisted_npm_shim_becomes_direct_node_argv_with_literal_symbols(self):
        shim, script = self.npm_shim()
        platform, which = self.windows()
        with platform, which:
            result = windows_commands.command_argv([str(shim), "--", "text & | < > ^ % ! \" spaced"], "codex")
        self.assertEqual(result, [str(self.node), str(script), "--", "text & | < > ^ % ! \" spaced"])
        self.assertNotIn("cmd.exe", result)

    def test_unknown_or_mismatched_batch_wrapper_fails_closed(self):
        shim, _ = self.npm_shim(command="cursor", package="@cursor/agent", bin_name="cursor")
        platform, which = self.windows()
        with platform, which, self.assertRaises(ValueError):
            windows_commands.command_argv([str(shim), "literal & input"], "cursor")
        shim, _ = self.npm_shim()
        platform, which = self.windows()
        with platform, which, self.assertRaises(ValueError):
            windows_commands.command_argv([str(shim)], "claude")

    def test_bad_metadata_escape_and_reparse_target_are_rejected(self):
        shim, script = self.npm_shim(target="../outside.js")
        outside = self.root / "outside.js"
        outside.write_text("outside", encoding="utf-8")
        platform, which = self.windows()
        with platform, which, self.assertRaises(ValueError):
            windows_commands.command_argv([str(shim)], "codex")

    def test_reparse_package_directory_is_rejected_before_metadata_read(self):
        shim, _ = self.npm_shim()
        package = self.root / "node_modules" / "@openai" / "codex"
        outside = self.root / "outside-package"
        package.rename(outside)
        package.symlink_to(outside, target_is_directory=True)
        platform, which = self.windows()
        with platform, which, self.assertRaises(ValueError):
            windows_commands.command_argv([str(shim)], "codex")
        shim, script = self.npm_shim(target="bin/link.js")
        script.unlink()
        script.symlink_to(outside)
        platform, which = self.windows()
        with platform, which, self.assertRaises(ValueError):
            windows_commands.command_argv([str(shim)], "codex")

    def test_non_windows_and_exe_commands_remain_plain_argv(self):
        self.assertEqual(windows_commands.command_argv(["codex", "--version"], "codex"), ["codex", "--version"])
        with patch.object(windows_commands, "WINDOWS", True):
            self.assertEqual(windows_commands.command_argv(["C:/tools/codex.exe", "--version"], "codex"),
                             ["C:/tools/codex.exe", "--version"])

    @unittest.skipUnless(windows_commands.WINDOWS, "CommandLineToArgvW requires Windows")
    def test_windows_split_preserves_backslash_paths_and_metacharacters_as_argv(self):
        command = '"C:\\Program Files\\Python\\python.exe" -c "print(\'&|<>^%!\')"'
        self.assertEqual(windows_commands.split_command(command),
                         [r"C:\Program Files\Python\python.exe", "-c", "print('&|<>^%!')"])


if __name__ == "__main__":
    unittest.main()
