#!/usr/bin/env python3
"""Install the accelerator as a desktop application: a hare in the application list, the browser on a click.

After cloning, run once from the clone:

    ./accelerator-app install      # AI Accelerator appears among the installed applications
    ./accelerator-app open         # what the icon runs: start the Harness if needed, open it in the browser
    ./accelerator-app status
    ./accelerator-app stop         # stop the Harness server
    ./accelerator-app update       # what the page's Update button does: update the clone and restart
    ./accelerator-app uninstall    # remove the application entry; projects and memory stay

Nothing is copied but the application entry and its icon: the entry runs this clone, so updating the clone
updates the application. The page shows an Update button when the branch the clone follows moves on (updates.py).
Per platform:

- Linux: `~/.local/share/applications/ai-accelerator.desktop` and the hicolor icons, which GNOME, KDE and
  every XDG desktop list among the applications;
- macOS: `~/Applications/AI Accelerator.app`, listed in Launchpad and Spotlight;
- Windows: a Start menu shortcut and an entry under Settings › Apps › Installed apps (with Uninstall).

Started from a desktop the launcher has none of the shell's PATH additions - `~/.local/bin`, nvm, Homebrew -
where the Claude, Codex and Cursor CLIs, and often Python itself, live. So the entry names the Python that
ran `install` (ACCELERATOR_APP_PYTHON; the launcher falls back to python3 and python). Before starting the
server, `open` reads the PATH the user's login shell sets up today, as VS Code does for a desktop launch, so a
CLI installed since `install` is found; the PATH `install` recorded is the fallback. A clone that was moved is
followed: started once from its new folder, the launcher points the installed application there.

Standard library only.
"""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import re
import shlex
import shutil
import stat
import struct
import subprocess
import sys
import time
import webbrowser
from pathlib import Path
from typing import Callable, Optional

ROOT = Path(__file__).resolve().parents[3]
ICONS = ROOT / 'harness' / 'web' / 'icons'
SERVER = ROOT / 'harness' / 'src' / 'harness' / 'web.py'
LAUNCHER = ROOT / 'accelerator-app'
SCRIPT = Path(__file__).resolve()
APP_ID = 'ai-accelerator'
APP_NAME = 'AI Accelerator'
APP_COMMENT = 'Choose a project folder and work with Claude Code, Codex or Cursor Agent in your browser'
APP_NAME_RU = 'AI Акселератор'
APP_COMMENT_RU = 'Выберите папку проекта и работайте с Claude Code, Codex или Cursor Agent в браузере'
BUNDLE_ID = 'local.ai-infrastructure.accelerator'
WINDOWS_KEY = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\AIAccelerator'
LINUX_SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
# Retina variants reuse the next size up, as Apple's own icon sets do.
ICNS_TYPES = (('icp4', 16), ('icp5', 32), ('icp6', 64), ('ic07', 128), ('ic08', 256), ('ic09', 512), ('ic10', 1024),
              ('ic11', 32), ('ic12', 64), ('ic13', 256), ('ic14', 512))
# Desktop Entry Specification, "The Exec key": an argument holding one of these must be quoted.
EXEC_RESERVED = set(' \t\n"\'\\><~|&;$*?#()`')
PYTHON_VARIABLE = 'ACCELERATOR_APP_PYTHON'
URL_PATTERN = re.compile(r'(?:Started|Already running): (http://127\.0\.0\.1:\d+)')
LOG_LIMIT = 256 * 1024
SHELL_SECONDS = 10
SHELL_MARK = '__HARNESS_PATH__'


class AppError(Exception):
    """A user-facing reason the application cannot be installed or opened."""


# ---------------------------------------------------------------------------
# Where things are

def platform_name() -> str:
    if sys.platform.startswith('linux'):
        return 'linux'
    if sys.platform == 'darwin':
        return 'macos'
    if os.name == 'nt':
        return 'windows'
    return 'other'


def config_dir(platform: Optional[str] = None) -> Path:
    """Where the launcher keeps the recorded PATH and its log; never the project's or the clone's."""
    platform = platform or platform_name()
    if platform == 'windows':
        return Path(os.environ.get('APPDATA') or Path.home() / 'AppData/Roaming') / 'ai-infrastructure-harness'
    if platform == 'macos':
        return Path.home() / 'Library/Application Support/ai-infrastructure-harness'
    return Path(os.environ.get('XDG_CONFIG_HOME') or Path.home() / '.config') / 'ai-infrastructure-harness'


def linux_data_home() -> Path:
    return Path(os.environ.get('XDG_DATA_HOME') or Path.home() / '.local/share')


def linux_paths() -> dict[str, Path]:
    data = linux_data_home()
    return {'entry': data / 'applications' / f'{APP_ID}.desktop', 'icons': data / 'icons' / 'hicolor'}


def macos_bundle() -> Path:
    return Path.home() / 'Applications' / f'{APP_NAME}.app'


def windows_paths() -> dict[str, Path]:
    programs = Path(os.environ.get('APPDATA') or Path.home() / 'AppData/Roaming') / 'Microsoft/Windows/Start Menu/Programs'
    return {'shortcut': programs / f'{APP_NAME}.lnk', 'icon': config_dir('windows') / f'{APP_ID}.ico'}


def png(size: int) -> Path:
    return ICONS / f'{APP_ID}-{size}.png'


def version() -> str:
    try:
        match = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / 'harness/pyproject.toml').read_text(encoding='utf-8'), re.M)
    except OSError:
        match = None
    return match.group(1) if match else '1.0.0'


# ---------------------------------------------------------------------------
# Icon containers: both formats may embed PNG images as they are

def ico_bytes(sizes=ICO_SIZES) -> bytes:
    images = [png(size).read_bytes() for size in sizes]
    header = struct.pack('<HHH', 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries, data = b'', b''
    for size, image in zip(sizes, images):
        # A 256-pixel side is written as 0: the field is one byte.
        entries += struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(image), offset + len(data))
        data += image
    return header + entries + data


def icns_bytes(types=ICNS_TYPES) -> bytes:
    body = b''.join(kind.encode('ascii') + struct.pack('>I', 8 + len(image)) + image
                    for kind, image in ((kind, png(size).read_bytes()) for kind, size in types))
    return b'icns' + struct.pack('>I', 8 + len(body)) + body


# ---------------------------------------------------------------------------
# Linux: an XDG desktop entry and hicolor icons

def exec_argument(value: str) -> str:
    """One Exec argument, quoted as the Desktop Entry Specification requires."""
    if any(char in EXEC_RESERVED for char in value):
        value = '"' + ''.join('\\' + char if char in '"`$\\' else char for char in value) + '"'
    return value.replace('%', '%%')


def desktop_string(value: str) -> str:
    """The specification's escape for string values, applied before the Exec quoting is read."""
    return value.replace('\\', '\\\\').replace('\n', '\\n').replace('\t', '\\t')


def launch_command(launcher: Path, python: str) -> str:
    """The launcher, told which Python ran `install`; it falls back to python3 and python when that one is gone."""
    return ' '.join(exec_argument(part) for part in ('env', f'{PYTHON_VARIABLE}={python}', str(launcher)))


def desktop_entry(launcher: Path = LAUNCHER, python: str = sys.executable) -> str:
    command = launch_command(launcher, python)
    return '\n'.join([
        '[Desktop Entry]',
        'Type=Application',
        'Version=1.5',
        f'Name={APP_NAME}',
        f'Name[ru]={APP_NAME_RU}',
        'GenericName=AI coding workspace',
        f'Comment={APP_COMMENT}',
        f'Comment[ru]={APP_COMMENT_RU}',
        f'Exec={desktop_string(command + " open")}',
        f'TryExec={desktop_string(str(launcher))}',
        f'Icon={APP_ID}',
        'Terminal=false',
        'Categories=Development;IDE;',
        'Keywords=AI;Accelerator;Claude;Codex;Cursor;PHP;Laravel;Symfony;WordPress;Harness;',
        'StartupNotify=false',
        'Actions=stop;',
        '',
        '[Desktop Action stop]',
        'Name=Stop the accelerator server',
        'Name[ru]=Остановить сервер акселератора',
        f'Exec={desktop_string(command + " stop")}',
        '',
    ])


def _quiet(command: list[str]) -> None:
    if shutil.which(command[0]):
        try:
            subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           timeout=60, check=False)
        except (OSError, subprocess.SubprocessError):
            pass


def linux_refresh(paths: dict[str, Path]) -> None:
    """Tell the desktop now rather than at the next login. GNOME watches the folders by itself."""
    _quiet(['update-desktop-database', '-q', str(paths['entry'].parent)])
    # A cache the user already has must not go stale; one that is absent is not created.
    if (paths['icons'] / 'icon-theme.cache').is_file():
        _quiet(['gtk-update-icon-cache', '-q', '-t', '-f', str(paths['icons'])])
    if 'KDE' in os.environ.get('XDG_CURRENT_DESKTOP', '').upper():
        for builder in ('kbuildsycoca6', 'kbuildsycoca5'):
            if shutil.which(builder):
                _quiet([builder])
                break


def install_linux(refresh: bool = True) -> list[Path]:
    paths = linux_paths()
    written = []
    scalable = paths['icons'] / 'scalable' / 'apps' / f'{APP_ID}.svg'
    scalable.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(ICONS / f'{APP_ID}.svg', scalable)
    written.append(scalable)
    for size in LINUX_SIZES:
        target = paths['icons'] / f'{size}x{size}' / 'apps' / f'{APP_ID}.png'
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(png(size), target)
        written.append(target)
    paths['entry'].parent.mkdir(parents=True, exist_ok=True)
    paths['entry'].write_text(desktop_entry(), encoding='utf-8')
    paths['entry'].chmod(0o755)  # some desktops only offer an executable entry as trusted
    written.append(paths['entry'])
    if refresh:
        linux_refresh(paths)
    return written


def uninstall_linux(refresh: bool = True) -> list[Path]:
    paths = linux_paths()
    candidates = [paths['entry'], paths['icons'] / 'scalable' / 'apps' / f'{APP_ID}.svg',
                  *(paths['icons'] / f'{size}x{size}' / 'apps' / f'{APP_ID}.png' for size in LINUX_SIZES)]
    removed = [path for path in candidates if path.is_file()]
    for path in removed:
        path.unlink()
    if refresh:
        linux_refresh(paths)
    return removed


# ---------------------------------------------------------------------------
# macOS: an application bundle in ~/Applications

def macos_info() -> dict:
    return {
        'CFBundleName': APP_NAME, 'CFBundleDisplayName': APP_NAME, 'CFBundleIdentifier': BUNDLE_ID,
        'CFBundleVersion': version(), 'CFBundleShortVersionString': version(), 'CFBundlePackageType': 'APPL',
        'CFBundleExecutable': APP_ID, 'CFBundleIconFile': 'AppIcon', 'LSMinimumSystemVersion': '11.0',
        # The launcher exits once the browser has the page: no Dock icon that bounces and vanishes.
        'LSUIElement': True, 'NSHighResolutionCapable': True,
    }


def install_macos() -> list[Path]:
    bundle = macos_bundle()
    contents = bundle / 'Contents'
    executable = contents / 'MacOS' / APP_ID
    icon = contents / 'Resources' / 'AppIcon.icns'
    for folder in (executable.parent, icon.parent):
        folder.mkdir(parents=True, exist_ok=True)
    (contents / 'Info.plist').write_bytes(plistlib.dumps(macos_info()))
    executable.write_text(f'#!/bin/sh\n{PYTHON_VARIABLE}={shlex.quote(sys.executable)}\nexport {PYTHON_VARIABLE}\n'
                          f'exec {shlex.quote(str(LAUNCHER))} open "$@"\n', encoding='utf-8')
    executable.chmod(0o755)
    icon.write_bytes(icns_bytes())
    register = Path('/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework'
                    '/Support/lsregister')
    if register.is_file():
        _quiet([str(register), '-f', str(bundle)])
    return [bundle]


def uninstall_macos() -> list[Path]:
    bundle = macos_bundle()
    if bundle.is_dir() and (bundle / 'Contents' / 'Info.plist').is_file():
        info = plistlib.loads((bundle / 'Contents' / 'Info.plist').read_bytes())
        if info.get('CFBundleIdentifier') != BUNDLE_ID:
            raise AppError(f'{bundle} belongs to another application; it was left in place.')
        shutil.rmtree(bundle)
        return [bundle]
    return []


# ---------------------------------------------------------------------------
# Windows: a Start menu shortcut and an Installed apps entry

def windows_python() -> Path:
    """pythonw runs the launcher without a console window flashing up."""
    current = Path(sys.executable)
    windowed = current.with_name('pythonw.exe')
    return windowed if windowed.is_file() else current


def powershell_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def shortcut_script(shortcut: Path, icon: Path, python: Path) -> str:
    arguments = f'"{SCRIPT}" open'
    return '; '.join([
        '$shell = New-Object -ComObject WScript.Shell',
        f'$link = $shell.CreateShortcut({powershell_literal(str(shortcut))})',
        f'$link.TargetPath = {powershell_literal(str(python))}',
        f'$link.Arguments = {powershell_literal(arguments)}',
        f'$link.WorkingDirectory = {powershell_literal(str(ROOT))}',
        f'$link.IconLocation = {powershell_literal(str(icon) + ",0")}',
        f'$link.Description = {powershell_literal(APP_COMMENT)}',
        '$link.Save()',
    ])


def registry_values(icon: Path, python: Path) -> dict[str, object]:
    return {
        'DisplayName': APP_NAME, 'DisplayIcon': str(icon), 'DisplayVersion': version(),
        'Publisher': 'AI Infrastructure', 'InstallLocation': str(ROOT), 'Comments': APP_COMMENT,
        'UninstallString': f'"{python}" "{SCRIPT}" uninstall', 'NoModify': 1, 'NoRepair': 1,
    }


def install_windows(registry=None, run=subprocess.run) -> list[Path]:
    paths = windows_paths()
    python = windows_python()
    paths['icon'].parent.mkdir(parents=True, exist_ok=True)
    paths['icon'].write_bytes(ico_bytes())
    paths['shortcut'].parent.mkdir(parents=True, exist_ok=True)
    result = run(['powershell', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command',
                  shortcut_script(paths['shortcut'], paths['icon'], python)],
                 capture_output=True, text=True, timeout=120, check=False)
    if result.returncode != 0:
        raise AppError('The Start menu shortcut could not be created: ' + (result.stderr or result.stdout).strip()[:400])
    if registry is None:
        import winreg as registry  # noqa: PLC0415 - Windows only
    with registry.CreateKeyEx(registry.HKEY_CURRENT_USER, WINDOWS_KEY, 0, registry.KEY_SET_VALUE) as key:
        for name, value in registry_values(paths['icon'], python).items():
            kind = registry.REG_DWORD if isinstance(value, int) else registry.REG_SZ
            registry.SetValueEx(key, name, 0, kind, value)
    return [paths['shortcut'], paths['icon']]


def uninstall_windows(registry=None) -> list[Path]:
    paths = windows_paths()
    removed = [path for path in (paths['shortcut'], paths['icon']) if path.is_file()]
    for path in removed:
        path.unlink()
    if registry is None:
        import winreg as registry  # noqa: PLC0415 - Windows only
    try:
        registry.DeleteKey(registry.HKEY_CURRENT_USER, WINDOWS_KEY)
    except OSError:
        pass
    return removed


# ---------------------------------------------------------------------------
# Install, uninstall, status

def save_config(shell_path_now: Optional[str] = None) -> Path:
    """Record what a desktop launch lacks: the shell's PATH, from the installing shell or as last read."""
    path = config_dir() / 'app.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    installed = load_config().get('installed_at') if shell_path_now else None
    path.write_text(json.dumps({'clone': str(ROOT), 'path': shell_path_now or os.environ.get('PATH', ''),
                                'installed_at': installed or time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}, indent=2) + '\n',
                    encoding='utf-8')
    return path


def load_config() -> dict:
    try:
        value = json.loads((config_dir() / 'app.json').read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return {}
    return value if isinstance(value, dict) else {}


def install(refresh: bool = True) -> list[Path]:
    for source in (ICONS / f'{APP_ID}.svg', *(png(size) for size in (*LINUX_SIZES, 1024))):
        if not source.is_file():
            raise AppError(f'The application icon is missing from the clone: {source}')
    platform = platform_name()
    if platform == 'linux':
        written = install_linux(refresh)
    elif platform == 'macos':
        written = install_macos()
    elif platform == 'windows':
        written = install_windows()
    else:
        raise AppError(f'Installing an application entry is not supported on {sys.platform}; run ./accelerator-app open.')
    save_config()
    return written


def uninstall(refresh: bool = True) -> list[Path]:
    platform = platform_name()
    if platform == 'linux':
        removed = uninstall_linux(refresh)
    elif platform == 'macos':
        removed = uninstall_macos()
    elif platform == 'windows':
        removed = uninstall_windows()
    else:
        removed = []
    try:
        (config_dir() / 'app.json').unlink()
    except OSError:
        pass
    return removed


def installed_entry() -> Optional[Path]:
    platform = platform_name()
    path = {'linux': lambda: linux_paths()['entry'], 'macos': macos_bundle,
            'windows': lambda: windows_paths()['shortcut']}.get(platform, lambda: None)()
    return path if path is not None and path.exists() else None


# ---------------------------------------------------------------------------
# Open: what a click on the icon runs

def shell_path(run=subprocess.run) -> Optional[str]:
    """The PATH the user's login shell sets up, read as VS Code reads it for a desktop launch: the shell runs as an
    interactive login shell and prints it between marks, so greetings and prompts around it do not matter. None on
    Windows, without a shell, or when the shell fails or takes too long."""
    shell = os.environ.get('SHELL', '')
    if os.name == 'nt' or not shell or not Path(shell).is_file():
        return None
    script = (f'{shlex.quote(sys.executable)} -c "import os, sys; '
              f"sys.stdout.write('{SHELL_MARK}' + os.environ.get('PATH', '') + '{SHELL_MARK}')\"")
    try:
        result = run([shell, '-ilc', script], stdin=subprocess.DEVNULL, capture_output=True, text=True,
                     timeout=SHELL_SECONDS, check=False)
    except (OSError, subprocess.SubprocessError):
        return None
    match = re.search(f'{SHELL_MARK}(.*?){SHELL_MARK}', result.stdout or '', re.S)
    return match.group(1) if match and match.group(1) else None


def launch_environment(config: Optional[dict] = None, fresh: Optional[str] = None) -> dict[str, str]:
    """The environment a desktop launch starts the server in: the shell's PATH as read now, then the one `install`
    recorded, then the desktop's own."""
    environment = dict(os.environ)
    recorded = (config if config is not None else load_config()).get('path')
    sources = (fresh, recorded if isinstance(recorded, str) else None, environment.get('PATH', ''))
    parts = [part for source in sources if source for part in source.split(os.pathsep) if part]
    if parts:
        environment['PATH'] = os.pathsep.join(dict.fromkeys(parts))
    return environment


def log(message: str) -> None:
    path = config_dir() / 'launcher.log'
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.is_file() and path.stat().st_size > LOG_LIMIT:
            path.write_text('', encoding='utf-8')
        with path.open('a', encoding='utf-8') as handle:
            handle.write(time.strftime('%Y-%m-%d %H:%M:%S ') + message.rstrip() + '\n')
    except OSError:
        pass


def say(message: str, stream=None) -> None:
    """Print when there is a terminal; pythonw, which the Windows shortcut runs, has no streams at all."""
    stream = stream or sys.stdout
    if stream is not None:
        try:
            print(message, file=stream)
        except (OSError, ValueError):
            pass


def notify(message: str) -> None:
    """A desktop launch has no terminal: say what went wrong where the person looks."""
    log(message)
    say(f'accelerator-app: {message}', sys.stderr)
    platform = platform_name()
    if platform == 'linux':
        _quiet(['notify-send', f'--app-name={APP_NAME}', f'--icon={APP_ID}', APP_NAME, message])
    elif platform == 'macos':
        text = message.replace('\\', '\\\\').replace('"', '\\"')
        _quiet(['osascript', '-e', f'display notification "{text}" with title "{APP_NAME}"'])
    elif platform == 'windows':
        try:
            import ctypes  # noqa: PLC0415 - Windows only
            ctypes.windll.user32.MessageBoxW(None, message, APP_NAME, 0x30)  # MB_ICONWARNING
        except (ImportError, AttributeError, OSError):
            pass


def server(command: str, run=subprocess.run, fresh: Optional[str] = None, timeout: int = 120) -> subprocess.CompletedProcess:
    # Run from the clone: `start` then registers no folder of its own, and the browser asks for a project.
    # CREATE_NO_WINDOW: a console Python started from the Start menu would otherwise flash a console window.
    extra = {'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}
    return run([sys.executable, str(SERVER), command], cwd=str(ROOT), env=launch_environment(fresh=fresh),
               stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=timeout, check=False, **extra)


def follow_moved_clone() -> Optional[str]:
    """Started from a clone after the one the installed application runs was moved away: point the application
    here. Another clone that still exists keeps the application."""
    clone = load_config().get('clone')
    if not isinstance(clone, str) or clone == str(ROOT) or (Path(clone) / LAUNCHER.name).is_file() or installed_entry() is None:
        return None
    install()
    return clone


def open_app(run=subprocess.run, browser: Callable[..., bool] = webbrowser.open, shell=shell_path) -> int:
    try:
        # A running server keeps its environment. A new one gets the shell's PATH as it is now, which then
        # replaces the recorded one: a CLI installed since `install` is found.
        fresh = shell(run) if server('status', run).returncode != 0 else None
        if fresh and load_config():
            save_config(fresh)
        result = server('start', run, fresh)
    except (OSError, subprocess.SubprocessError) as error:
        notify(f'The accelerator could not start: {error}')
        return 1
    match = URL_PATTERN.search(result.stdout or '')
    if result.returncode != 0 or match is None:
        detail = (result.stderr or result.stdout or '').strip().splitlines()
        notify('The accelerator could not start. ' + (detail[-1] if detail else 'See the Harness server log.'))
        return 1
    url = match.group(1) + '/'
    log(f'open {url}')
    if not browser(url, new=2):
        notify(f'Open {url} in your browser.')
    else:
        say(f'Opened {url}')
    return 0


# ---------------------------------------------------------------------------
# Command line

def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog='accelerator-app', description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=('install', 'uninstall', 'open', 'status', 'stop', 'update'))
    options = parser.parse_args(argv)
    try:
        if options.command not in ('install', 'uninstall'):
            moved = follow_moved_clone()
            if moved:
                log(f'The application now runs {ROOT} (it ran {moved}).')
                say(f'{APP_NAME} now runs this clone, {ROOT}; it ran {moved}.')
        if options.command == 'install':
            written = install()
            print(f'Installed {APP_NAME} from {ROOT}:')
            for path in written:
                print(f'  {path}')
            print(f'Find "{APP_NAME}" among your applications; it opens the accelerator in your browser.')
            print('It keeps itself current: when the branch this clone follows moves on, the page shows an Update button.')
            print('Moved the clone? Start it once from the new folder (./accelerator-app open) and the application follows.')
            return 0
        if options.command == 'uninstall':
            removed = uninstall()
            print(f'Removed {APP_NAME}' + (':' if removed else ' (it was not installed).'))
            for path in removed:
                print(f'  {path}')
            print('Projects, sessions and their memory are kept; ./accelerator-app stop stops a running server.')
            return 0
        if options.command == 'open':
            return open_app()
        if options.command == 'update':
            result = server('update', timeout=600)
            say((result.stdout or '').strip())
            if result.returncode:
                print((result.stderr or 'The update did not finish.').strip(), file=sys.stderr)
            return result.returncode
        if options.command == 'status':
            entry = installed_entry()
            print(f'Application: {"installed at " + str(entry) if entry else "not installed (./accelerator-app install)"}')
            result = server('status')
            print(f'Server: {(result.stdout or result.stderr).strip() or "unknown"}')
            return 0
        result = server('stop')
        text = (result.stdout or result.stderr).strip()
        # The stop action of the application entry also runs without a terminal.
        if result.returncode:
            notify(text or 'The accelerator server did not stop.')
        else:
            say(text)
        return result.returncode
    except (AppError, OSError) as error:
        print(f'accelerator-app: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
