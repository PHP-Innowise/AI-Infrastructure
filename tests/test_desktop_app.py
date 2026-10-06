"""accelerator-app: the hare among the installed applications, the browser on a click.

Every test writes into temporary directories (XDG folders, HOME, APPDATA) and fakes what reaches the
system - the desktop caches, PowerShell, the Windows registry, the browser - so nothing is installed here.
"""

import contextlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import shlex
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "harness/src"))
from harness import desktop_app as app  # noqa: E402

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def png_size(data):
    return struct.unpack(">II", data[16:24])


class IconTests(unittest.TestCase):
    def test_every_size_the_platforms_need_is_a_png_of_that_size(self):
        sizes = sorted({*app.LINUX_SIZES, *app.ICO_SIZES, *(size for _, size in app.ICNS_TYPES)})
        for size in sizes:
            with self.subTest(size=size):
                data = app.png(size).read_bytes()
                self.assertTrue(data.startswith(PNG_SIGNATURE))
                self.assertEqual((size, size), png_size(data))

    def test_the_svg_master_is_the_named_application(self):
        svg = (app.ICONS / "ai-accelerator.svg").read_text(encoding="utf-8")
        self.assertIn("<title>AI Accelerator</title>", svg)
        self.assertIn('viewBox="0 0 512 512"', svg)

    def test_ico_embeds_every_png_in_a_valid_directory(self):
        data = app.ico_bytes()
        reserved, kind, count = struct.unpack("<HHH", data[:6])
        self.assertEqual((0, 1, len(app.ICO_SIZES)), (reserved, kind, count))
        for index, size in enumerate(app.ICO_SIZES):
            width, height, _, _, planes, bits, length, offset = struct.unpack("<BBBBHHII", data[6 + 16 * index:22 + 16 * index])
            self.assertEqual((size % 256, size % 256, 1, 32), (width, height, planes, bits))
            image = data[offset:offset + length]
            self.assertTrue(image.startswith(PNG_SIGNATURE))
            self.assertEqual((size, size), png_size(image))

    def test_icns_is_a_well_formed_container(self):
        data = app.icns_bytes()
        self.assertEqual(b"icns", data[:4])
        self.assertEqual(len(data), struct.unpack(">I", data[4:8])[0])
        position, seen = 8, []
        while position < len(data):
            kind, length = data[position:position + 4].decode("ascii"), struct.unpack(">I", data[position + 4:position + 8])[0]
            self.assertTrue(data[position + 8:position + length].startswith(PNG_SIGNATURE))
            seen.append(kind)
            position += length
        self.assertEqual([kind for kind, _ in app.ICNS_TYPES], seen)


class DesktopEntryTests(unittest.TestCase):
    def test_exec_arguments_are_quoted_as_the_specification_requires(self):
        self.assertEqual("/home/u/ai-accelerator/accelerator-app", app.exec_argument("/home/u/ai-accelerator/accelerator-app"))
        self.assertEqual('"/home/u/My Tools/\\$x\\`/accelerator-app"', app.exec_argument("/home/u/My Tools/$x`/accelerator-app"))
        self.assertEqual("/home/u/100%%/accelerator-app", app.exec_argument("/home/u/100%/accelerator-app"))
        # The string escape is read before the quoting: a backslash inside quotes becomes four.
        self.assertEqual('"C:\\\\\\\\tools"', app.desktop_string(app.exec_argument("C:\\tools")))

    def test_the_entry_runs_this_clone_with_the_python_that_installed_it(self):
        entry = app.desktop_entry(PurePosixPath("/home/u/ai accelerator/accelerator-app"), "/opt/homebrew/bin/python3")
        command = 'env ACCELERATOR_APP_PYTHON=/opt/homebrew/bin/python3 "/home/u/ai accelerator/accelerator-app"'
        self.assertIn(f"Exec={command} open\n", entry)
        self.assertIn(f"Exec={command} stop\n", entry)
        self.assertIn("TryExec=/home/u/ai accelerator/accelerator-app\n", entry)
        self.assertIn("Icon=ai-accelerator\n", entry)
        self.assertIn("Name=AI Accelerator\n", entry)
        self.assertIn("Terminal=false\n", entry)

    @unittest.skipUnless(shutil.which("desktop-file-validate"), "desktop-file-validate is not installed")
    def test_the_entry_passes_desktop_file_validate(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "ai-accelerator.desktop"
            path.write_text(app.desktop_entry(PurePosixPath("/opt/My Tools/accelerator-app"), "/opt/py 3/bin/python3"), encoding="utf-8")
            result = subprocess.run(["desktop-file-validate", str(path)], capture_output=True, text=True)
            self.assertEqual(0, result.returncode, result.stdout + result.stderr)


class Isolated(unittest.TestCase):
    platform = "linux"

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "home").mkdir()
        self.real_path = os.environ.get("PATH", "")
        # USERPROFILE: Path.home() reads it, not HOME, on Windows.
        environment = {"HOME": str(self.root / "home"), "USERPROFILE": str(self.root / "home"),
                       "XDG_DATA_HOME": str(self.root / "data"),
                       "XDG_CONFIG_HOME": str(self.root / "config"), "APPDATA": str(self.root / "appdata"),
                       "PATH": os.pathsep.join(["/opt/agents/bin", "/usr/bin"])}
        patched = mock.patch.dict(os.environ, environment)
        patched.start()
        self.addCleanup(patched.stop)
        for name, value in (("platform_name", mock.Mock(return_value=self.platform)), ("notify", mock.Mock()),
                            ("_quiet", mock.Mock())):
            patch = mock.patch.object(app, name, value)
            patch.start()
            self.addCleanup(patch.stop)

    def files(self):
        return sorted(str(path.relative_to(self.root)) for path in self.root.rglob("*") if path.is_file())


@unittest.skipIf(os.name == "nt", "the entry's executable bit has no meaning on Windows")
class LinuxInstallTests(Isolated):
    def test_install_writes_the_entry_and_the_icons_and_uninstall_removes_them(self):
        app.install(refresh=False)
        entry = self.root / "data/applications/ai-accelerator.desktop"
        launcher = app.desktop_string(app.launch_command(app.LAUNCHER, sys.executable))
        self.assertIn(f"Exec={launcher} open\n", entry.read_text(encoding="utf-8"))
        self.assertTrue(entry.stat().st_mode & stat.S_IXUSR)
        self.assertTrue((self.root / "data/icons/hicolor/scalable/apps/ai-accelerator.svg").is_file())
        for size in app.LINUX_SIZES:
            icon = self.root / f"data/icons/hicolor/{size}x{size}/apps/ai-accelerator.png"
            self.assertEqual((size, size), png_size(icon.read_bytes()))
        recorded = json.loads((self.root / "config/ai-infrastructure-harness/app.json").read_text(encoding="utf-8"))
        self.assertEqual(os.environ["PATH"], recorded["path"])
        self.assertEqual(str(app.ROOT), recorded["clone"])
        self.assertEqual(entry, app.installed_entry())

        app.uninstall(refresh=False)
        self.assertEqual([], self.files())
        self.assertIsNone(app.installed_entry())

    def test_install_refuses_a_clone_without_its_icons(self):
        with mock.patch.object(app, "ICONS", self.root / "missing"):
            with self.assertRaises(app.AppError):
                app.install(refresh=False)
        self.assertEqual([], self.files())


@unittest.skipIf(os.name == "nt", "the bundle's executable bit has no meaning on Windows")
class MacOSInstallTests(Isolated):
    platform = "macos"

    def test_install_builds_an_application_bundle(self):
        app.install()
        bundle = self.root / "home/Applications/AI Accelerator.app/Contents"
        info = plistlib.loads((bundle / "Info.plist").read_bytes())
        self.assertEqual(("AI Accelerator", app.BUNDLE_ID, "ai-accelerator", "AppIcon"),
                         (info["CFBundleName"], info["CFBundleIdentifier"], info["CFBundleExecutable"], info["CFBundleIconFile"]))
        executable = bundle / "MacOS/ai-accelerator"
        self.assertTrue(executable.stat().st_mode & stat.S_IXUSR)
        script = executable.read_text(encoding="utf-8")
        self.assertIn(f"ACCELERATOR_APP_PYTHON={shlex.quote(sys.executable)}\nexport ACCELERATOR_APP_PYTHON\n", script)
        self.assertIn(f"exec {shlex.quote(str(app.LAUNCHER))} open", script)
        self.assertEqual(b"icns", (bundle / "Resources/AppIcon.icns").read_bytes()[:4])
        self.assertTrue((self.root / "home/Library/Application Support/ai-infrastructure-harness/app.json").is_file())

        app.uninstall()
        self.assertFalse((self.root / "home/Applications/AI Accelerator.app").exists())

    def test_uninstall_leaves_another_application_of_the_same_name(self):
        bundle = self.root / "home/Applications/AI Accelerator.app/Contents"
        bundle.mkdir(parents=True)
        (bundle / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.example.other"}))
        with self.assertRaises(app.AppError):
            app.uninstall()
        self.assertTrue((bundle / "Info.plist").is_file())


class FakeRegistry:
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, REG_DWORD = "HKCU", 2, 1, 4

    def __init__(self):
        self.values, self.deleted = {}, []

    def CreateKeyEx(self, root, key, reserved, access):
        registry = self

        class Key:
            def __enter__(self):
                return (root, key)

            def __exit__(self, *_):
                return False
        registry.created = (root, key)
        return Key()

    def SetValueEx(self, key, name, reserved, kind, value):
        self.values[name] = (kind, value)

    def DeleteKey(self, root, key):
        self.deleted.append((root, key))


class WindowsInstallTests(Isolated):
    platform = "windows"

    def test_install_creates_a_start_menu_shortcut_and_an_installed_apps_entry(self):
        registry, commands = FakeRegistry(), []
        run = mock.Mock(side_effect=lambda command, **_: commands.append(command) or subprocess.CompletedProcess(command, 0, "", ""))
        app.install_windows(registry=registry, run=run)
        script = commands[0][-1]
        self.assertEqual("powershell", commands[0][0])
        self.assertIn("CreateShortcut(", script)
        self.assertIn("AI Accelerator.lnk", script)
        self.assertIn(f"\"{app.SCRIPT}\" open", script)
        icon = self.root / "appdata/ai-infrastructure-harness/ai-accelerator.ico"
        self.assertEqual(b"\x00\x00\x01\x00", icon.read_bytes()[:4])
        self.assertEqual(("HKCU", app.WINDOWS_KEY), registry.created)
        self.assertEqual("AI Accelerator", registry.values["DisplayName"][1])
        self.assertEqual(str(icon), registry.values["DisplayIcon"][1])
        self.assertIn("uninstall", registry.values["UninstallString"][1])
        self.assertEqual((FakeRegistry.REG_DWORD, 1), registry.values["NoModify"])

        app.uninstall_windows(registry=registry)
        self.assertFalse(icon.exists())
        self.assertEqual([("HKCU", app.WINDOWS_KEY)], registry.deleted)

    @unittest.skipUnless(os.name == "nt", "needs PowerShell and the WScript.Shell COM object")
    def test_powershell_writes_a_shortcut_that_opens_the_accelerator(self):
        os.environ["PATH"] = self.real_path  # where powershell.exe is found
        app.install_windows(registry=FakeRegistry())
        shortcut = self.root / "appdata/Microsoft/Windows/Start Menu/Programs/AI Accelerator.lnk"
        self.assertTrue(shortcut.is_file())
        script = (f"$link = (New-Object -ComObject WScript.Shell).CreateShortcut({app.powershell_literal(str(shortcut))}); "
                  "$link.TargetPath; $link.Arguments; $link.IconLocation; $link.WorkingDirectory")
        result = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
                                capture_output=True, text=True, timeout=120)
        self.assertEqual(0, result.returncode, result.stderr)
        target, arguments, icon, folder = result.stdout.splitlines()[:4]

        def same(left, right):
            # The temporary folder may be named in 8.3 form on one side and in full on the other.
            return os.path.normcase(os.path.realpath(left)) == os.path.normcase(os.path.realpath(right))
        self.assertTrue(same(app.windows_python(), target), target)
        self.assertEqual(f'"{app.SCRIPT}" open', arguments)
        location, index = icon.rsplit(",", 1)
        self.assertTrue(same(self.root / "appdata/ai-infrastructure-harness/ai-accelerator.ico", location), icon)
        self.assertEqual("0", index)
        self.assertTrue(same(app.ROOT, folder), folder)

    def test_a_quote_in_a_path_cannot_end_the_powershell_string(self):
        self.assertEqual("'C:\\Users\\O''Brien\\x.lnk'", app.powershell_literal("C:\\Users\\O'Brien\\x.lnk"))

    def test_a_refused_shortcut_is_reported(self):
        run = mock.Mock(return_value=subprocess.CompletedProcess([], 1, "", "Access denied"))
        with self.assertRaises(app.AppError):
            app.install_windows(registry=FakeRegistry(), run=run)


class OpenTests(Isolated):
    def test_a_click_starts_the_harness_from_the_clone_with_the_recorded_path(self):
        app.save_config()
        calls, opened = [], []
        os.environ["PATH"] = "/usr/bin"  # what a desktop launch gets

        def run(command, **options):
            calls.append((command, options))
            return subprocess.CompletedProcess(command, 0, "Started: http://127.0.0.1:8766\n", "")
        printed = io.StringIO()
        with contextlib.redirect_stdout(printed):
            self.assertEqual(0, app.open_app(run=run, browser=lambda url, new: opened.append((url, new)) or True))
        self.assertEqual("Opened http://127.0.0.1:8766/\n", printed.getvalue())
        command, options = calls[0]
        self.assertEqual([str(app.SERVER), "start"], command[1:])
        self.assertEqual(str(app.ROOT), options["cwd"])
        self.assertEqual(["/opt/agents/bin", "/usr/bin"], options["env"]["PATH"].split(os.pathsep))
        self.assertEqual([("http://127.0.0.1:8766/", 2)], opened)

    def test_a_running_harness_is_reused(self):
        run = mock.Mock(return_value=subprocess.CompletedProcess([], 0, "Already running: http://127.0.0.1:9100 (stop before changing projects or options)\n", ""))
        opened = []
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(0, app.open_app(run=run, browser=lambda url, new: opened.append(url) or True))
        self.assertEqual(["http://127.0.0.1:9100/"], opened)

    def test_a_harness_that_does_not_start_is_reported_and_no_page_opens(self):
        run = mock.Mock(return_value=subprocess.CompletedProcess([], 1, "", "harness-server: Server could not start. See server.log.\n"))
        browser = mock.Mock()
        self.assertEqual(1, app.open_app(run=run, browser=browser))
        browser.assert_not_called()
        app.notify.assert_called_once()
        self.assertIn("Server could not start", app.notify.call_args.args[0])


@unittest.skipIf(os.name == "nt", "Windows runs the root launcher through Git Bash")
class LauncherTests(unittest.TestCase):
    def test_the_root_launcher_runs_the_module(self):
        result = subprocess.run([str(ROOT / "accelerator-app"), "--help"], capture_output=True, text=True, timeout=60)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("install", result.stdout)

    def test_the_launcher_prefers_the_python_the_entry_names(self):
        with tempfile.TemporaryDirectory() as folder:
            # A desktop's PATH whose python3 and python are too old for the Harness; the refusal's
            # notification goes to a stub, not to the desktop of whoever runs the tests.
            for name, status in (("python3", 1), ("python", 1), ("notify-send", 0), ("osascript", 0)):
                stub = Path(folder) / name
                stub.write_text(f"#!/bin/sh\nexit {status}\n", encoding="utf-8")
                stub.chmod(0o755)
            environment = {**os.environ, "PATH": folder + os.pathsep + os.environ.get("PATH", "")}
            environment.pop("ACCELERATOR_APP_PYTHON", None)
            command = [shutil.which("bash"), str(ROOT / "accelerator-app"), "--help"]
            refused = subprocess.run(command, capture_output=True, text=True, timeout=60, env=environment)
            self.assertEqual(127, refused.returncode)
            self.assertIn("Python 3.10+ is required", refused.stderr)
            named = subprocess.run(command, capture_output=True, text=True, timeout=60,
                                   env={**environment, "ACCELERATOR_APP_PYTHON": sys.executable})
            self.assertEqual(0, named.returncode, named.stderr)


if __name__ == "__main__":
    unittest.main()
