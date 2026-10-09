#!/usr/bin/env python3
"""Portable project MCP registration; stdlib only, preserving other servers."""
from __future__ import annotations

import argparse
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

PATHS = {'claude': '.mcp.json', 'cursor': '.cursor/mcp.json', 'codex': '.codex/config.toml'}
BEGIN = '# BEGIN HARNESS MEMORY MCP'
END = '# END HARNESS MEMORY MCP'
BOOTSTRAP = '''# Harness memory bootstrap v1
import os, sys, runpy
from pathlib import Path
host = sys.argv[1]
if host not in ('claude', 'codex'):
    raise SystemExit('Unsupported memory host')
marker = '.mcp.json' if host == 'claude' else '.codex/config.toml'
start = (Path(os.environ.get('CLAUDE_PROJECT_DIR') or os.getcwd()) if host == 'claude' else Path.cwd()).resolve()
root = next((p for p in (start, *start.parents) if (p / marker).is_file()), None)
if root is None:
    raise SystemExit('Project MCP configuration not found')
script = root / 'memory-bank/scripts/mcp_server.py'
if not script.is_file():
    raise SystemExit('Install the memory runtime in this project')
sys.path.insert(0, str(script.parent))
sys.argv = [str(script), '--root', str(root), '--host', host]
runpy.run_path(str(script), run_name='__main__')
'''.strip()


class ConfigError(ValueError):
    pass


def python_command():
    candidates = [('python', []), ('py', ['-3']), ('python3', [])] if os.name == 'nt' else [('python3', []), ('python', [])]
    probe = 'import sys,sqlite3; assert sys.version_info >= (3,9); sqlite3.connect(":memory:").execute("CREATE VIRTUAL TABLE probe USING fts5(value)")'
    for command, prefix in candidates:
        try:
            result = subprocess.run([command, *prefix, '-c', probe], stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
        except (OSError, subprocess.SubprocessError):
            continue
        if result.returncode == 0:
            return command, prefix
    raise ConfigError('Memory MCP needs Python 3.9+ with SQLite FTS5 on the client PATH')


def entry(host, python=('python3', [])):
    command, prefix = python
    args = (['${workspaceFolder}/memory-bank/scripts/mcp_server.py', '--root', '${workspaceFolder}', '--host', host]
        if host == 'cursor' else ['-c', BOOTSTRAP, host])
    return {'type': 'stdio', 'command': command, 'args': [*prefix, *args]}


def codex_block(python=('python3', [])):
    value = entry('codex', python)
    return (BEGIN + '\n[mcp_servers.harness_memory]\ncommand = ' + json.dumps(value['command'])
        + '\nargs = ' + json.dumps(value['args']) + '\nenabled = true\n' + END + '\n')


def template(host):
    if host == 'codex': return codex_block().encode()
    return (json.dumps({'mcpServers': {'harness-memory': entry(host)}}, indent=2) + '\n').encode()


def _json(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result: raise ConfigError('Duplicate MCP configuration key')
            result[key] = value
        return result
    try:
        value = json.loads(raw, object_pairs_hook=unique,
            parse_constant=lambda _: (_ for _ in ()).throw(ConfigError('Nonfinite MCP JSON value')))
    except (ValueError, UnicodeError) as error:
        raise ConfigError('Invalid MCP JSON configuration') from error
    if not isinstance(value, dict) or not isinstance(value.get('mcpServers', {}), dict):
        raise ConfigError('MCP configuration must contain a server object')
    return value


PYTHONS = (('python3', []), ('python', []), ('py', ['-3']))
BOOTSTRAP_TAG = re.compile(r'# Harness memory bootstrap v[0-9]+\n')


def _launcher(command, args, host):
    """Any versioned bootstrap is ours, so a launcher fix can replace an older entry."""
    for name, prefix in PYTHONS:
        if command != name or not isinstance(args, list) or args[:len(prefix)] != prefix:
            continue
        rest = args[len(prefix):]
        if host == 'cursor':
            return rest == entry(host, (name, []))['args']
        return (len(rest) == 3 and rest[0] == '-c' and isinstance(rest[1], str)
                and BOOTSTRAP_TAG.match(rest[1]) is not None and rest[2] == host)
    return False


def _owned(value, host):
    return (isinstance(value, dict) and set(value) == {'type', 'command', 'args'} and value.get('type') == 'stdio'
            and _launcher(value.get('command'), value.get('args'), host))


def _owned_block(block):
    match = re.fullmatch(r'\n\[mcp_servers\.harness_memory\]\ncommand = (".*")\nargs = (\[.*\])\nenabled = true\n', block)
    if not match: return False
    try: return _launcher(json.loads(match[1]), json.loads(match[2]), 'codex')
    except ValueError: return False


KEY = r'(?:[A-Za-z0-9_-]+|"(?:\\.|[^"\\])*"|\'[^\']*\')'
KEY_PATH = re.compile(r'\s*' + KEY + r'(?:\s*\.\s*' + KEY + r')*\s*')
KEY_PART = re.compile(KEY)


def _keys(value):
    if not KEY_PATH.fullmatch(value): raise ConfigError('Unsupported TOML key')
    for part in KEY_PART.findall(value):
        if part.startswith(('"', "'")): _toml_value(part)
    return tuple(json.loads(p) if p.startswith('"') else p[1:-1] if p.startswith("'") else p
        for p in KEY_PART.findall(value))


def _toml_value(text):
    """Validate a conservative TOML value subset on Python 3.9/3.10."""
    index = 0

    def fail(): raise ConfigError('Invalid or unsupported Codex TOML value')

    def space(inline=False):
        nonlocal index
        while index < len(text):
            if text[index] in ' \t\r\n':
                if inline and text[index] in '\r\n': fail()
                index += 1
            elif text[index] == '#':
                if inline: fail()
                newline = text.find('\n', index)
                index = len(text) if newline < 0 else newline + 1
            else: break

    def value(depth=0):
        nonlocal index
        if depth > 64 or index >= len(text): fail()
        char = text[index]
        if char in ('"', "'"):
            multiline = text.startswith(char * 3, index)
            delimiter = char * (3 if multiline else 1)
            index += len(delimiter)
            while index < len(text):
                if text.startswith(delimiter, index):
                    index += len(delimiter); return
                current = text[index]
                if ord(current) < 32 or ord(current) == 127:
                    if current != '\t' and not (multiline and current == '\n') and not (
                        multiline and current == '\r' and text[index:index+2] == '\r\n'): fail()
                if char == '"' and current == '\\':
                    index += 1
                    if index >= len(text): fail()
                    escaped = text[index]
                    if multiline:
                        continuation = re.match(r'[ \t]*\r?\n[ \t\r\n]*', text[index:])
                        if continuation:
                            index += continuation.end(); continue
                    if escaped in 'btnfr"\\': index += 1; continue
                    if escaped in ('u', 'U'):
                        digits = 4 if escaped == 'u' else 8
                        scalar = text[index+1:index+1+digits]
                        if len(scalar) != digits or not re.fullmatch('[0-9A-Fa-f]+', scalar): fail()
                        codepoint = int(scalar, 16)
                        if codepoint > 0x10ffff or 0xd800 <= codepoint <= 0xdfff: fail()
                        index += 1 + digits; continue
                    fail()
                index += 1
            fail()
        if char == '[':
            index += 1; space()
            if index < len(text) and text[index] == ']': index += 1; return
            while True:
                value(depth + 1); space()
                if index >= len(text): fail()
                if text[index] == ']': index += 1; return
                if text[index] != ',': fail()
                index += 1; space()
                if index < len(text) and text[index] == ']': index += 1; return
        if char == '{':
            index += 1; space(inline=True); keys = set()
            if index < len(text) and text[index] == '}': index += 1; return
            while True:
                match = KEY_PATH.match(text, index)
                if not match: fail()
                key = _keys(match.group())
                if any(key[:len(old)] == old or old[:len(key)] == key for old in keys): fail()
                keys.add(key); index = match.end(); space(inline=True)
                if index >= len(text) or text[index] != '=': fail()
                index += 1; space(inline=True); value(depth + 1); space(inline=True)
                if index >= len(text): fail()
                if text[index] == '}': index += 1; return
                if text[index] != ',': fail()
                index += 1; space(inline=True)
        # RFC3339 date/time atoms may contain one space between date and time.
        date_match = re.match(r'\d{4}-\d{2}-\d{2}(?:[Tt ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:[Zz]|[+-]\d{2}:\d{2})?)?', text[index:])
        time_match = re.match(r'\d{2}:\d{2}:\d{2}(?:\.\d+)?', text[index:])
        match = date_match or time_match
        if match:
            atom = match.group()
            try:
                if date_match and len(atom) == 10: datetime.date.fromisoformat(atom)
                elif date_match: datetime.datetime.fromisoformat(atom.replace('t', 'T').replace('Z', '+00:00').replace('z', '+00:00'))
                else: datetime.time.fromisoformat(atom)
            except ValueError: fail()
            index += match.end(); return
        match = re.match(r'[^\s,#\]\}]+', text[index:])
        if not match: fail()
        atom = match.group()
        digit = r'[0-9](?:_?[0-9])*'
        decimal = r'(?:0|[1-9](?:_?[0-9])*)'
        number = (r'[+-]?' + decimal + r'(?:\.' + digit + r')?(?:[eE][+-]?' + digit + r')?'
            + r'|0x[0-9A-Fa-f](?:_?[0-9A-Fa-f])*|0o[0-7](?:_?[0-7])*|0b[01](?:_?[01])*|[+-]?(?:inf|nan)')
        if atom not in ('true', 'false') and not re.fullmatch(number, atom): fail()
        index += match.end()

    space(); value(); space()
    if index != len(text): fail()


def _check_toml(text, *, allow_memory=False):
    """Inspect keys outside strings/arrays; never rewrite unrelated TOML."""
    fallback = False
    try:
        import tomllib
    except ImportError:  # Python 3.9/3.10: conservative lexical fallback below.
        fallback = True
    else:
        try: tomllib.loads(text)
        except ValueError as error: raise ConfigError('Invalid Codex TOML configuration') from error
    table, quote, depth, seen, arrays, pending, kinds = (), None, 0, set(), {}, [], {}

    def scope(full, *, parents_only=False):
        return (tuple((prefix, count) for prefix, count in arrays.items()
            if full[:len(prefix)] == prefix and (not parents_only or len(prefix) < len(full))), full)

    for line in text.splitlines():
        if quote is None and depth == 0 and line.strip() and not line.lstrip().startswith('#'):
            if line.lstrip().startswith('['):
                match = re.fullmatch(r'\s*(\[\[?)(.*?)(\]\]?)\s*(?:#.*)?', line)
                if not match or len(match[1]) != len(match[3]): raise ConfigError('Unsupported Codex TOML table')
                table = _keys(match[2])
                if (table[:2] == ('mcp_servers', 'harness_memory') and
                    not (allow_memory and table == ('mcp_servers', 'harness_memory') and match[1] == '[')) or (table == ('mcp_servers',) and match[1] == '[['):
                    raise ConfigError('Memory MCP name belongs to another server')
                if fallback:
                    identity = scope(table, parents_only=True)
                    kind = 'array' if match[1] == '[[' else 'table'
                    if (identity in kinds and not (kind == kinds[identity] == 'array')) or any(
                        kinds.get(scope(table[:length], parents_only=True)) == 'scalar' for length in range(1, len(table))):
                        raise ConfigError('Duplicate or conflicting Codex TOML table')
                    kinds[identity] = kind
                if match[1] == '[[': arrays[table] = arrays.get(table, 0) + 1
                continue
            key, separator, rhs = line.partition('=')
            if not separator: raise ConfigError('Unsupported Codex TOML entry')
            key_parts = _keys(key)
            full = table + key_parts
            if full == ('mcp_servers',) or (full[:2] == ('mcp_servers', 'harness_memory') and
                    not (allow_memory and len(full) == 3 and full[-1] in ('command', 'args', 'enabled'))):
                raise ConfigError('Memory MCP name belongs to another server or an inline table')
            identity = scope(full)
            if identity in seen: raise ConfigError('Duplicate Codex TOML key')
            seen.add(identity)
            if fallback:
                if identity in kinds: raise ConfigError('Conflicting Codex TOML key')
                for length in range(1, len(key_parts)):
                    parent_identity = scope(table + key_parts[:length])
                    if kinds.get(parent_identity, 'dotted') != 'dotted':
                        raise ConfigError('Conflicting dotted Codex TOML key')
                    kinds[parent_identity] = 'dotted'
                kinds[identity] = 'scalar'
                pending = [rhs]
        elif fallback and pending:
            pending.append(line)
        index = 0
        while index < len(line):
            char = line[index]
            if quote is not None:
                if quote.startswith('"') and char == '\\': index += 2; continue
                if line.startswith(quote, index): index += len(quote); quote = None; continue
            elif char == '#': break
            elif char in ('"', "'"):
                quote = char * 3 if line.startswith(char * 3, index) else char
                index += len(quote); continue
            elif char in '[{': depth += 1
            elif char in ']}': depth -= 1
            if depth < 0: raise ConfigError('Invalid Codex TOML nesting')
            index += 1
        if quote is not None and len(quote) == 1:
            raise ConfigError('Invalid Codex TOML string')
        if fallback and quote is None and depth == 0 and pending:
            _toml_value('\n'.join(pending)); pending = []
    if quote is not None or depth != 0: raise ConfigError('Incomplete Codex TOML value')


def merge(path, previous, source, python=('python3', [])):
    host = next((host for host, name in PATHS.items() if name == path), None)
    if host is None: raise ConfigError('Unknown project MCP configuration')
    if host != 'codex':
        desired = _json(source)
        if not _owned(desired.get('mcpServers', {}).get('harness-memory'), host):
            raise ConfigError('Invalid shipped memory MCP entry')
        base = _json(previous) if previous is not None else desired
        servers = base.setdefault('mcpServers', {})
        old = servers.get('harness-memory')
        if 'harness-memory' in servers and not _owned(old, host):
            raise ConfigError('Memory MCP name belongs to another server')
        servers['harness-memory'] = entry(host, python)
        return (json.dumps(base, indent=2, ensure_ascii=False, allow_nan=False) + '\n').encode()
    text = (previous if previous is not None else source).decode('utf-8')
    if text.count(BEGIN) != text.count(END) or text.count(BEGIN) > 1:
        raise ConfigError('Malformed managed memory MCP block')
    if BEGIN in text:
        before, rest = text.split(BEGIN, 1)
        block, after = rest.split(END, 1)
        if not _owned_block(block):
            raise ConfigError('Managed memory MCP block was edited; reconcile it before installing')
        _check_toml(before + after)
        result = before + codex_block(python).rstrip('\n') + after
    else:
        _check_toml(text)
        result = text + ('\n' if text and not text.endswith('\n') else '') + '\n' + codex_block(python)
    _check_toml(result, allow_memory=True)
    return result.encode('utf-8')


def _confined(root, target):
    for part in (target, *target.parents):
        if part == root: break
        if part.is_symlink(): raise ConfigError('MCP configuration path contains a symlink')
        if part.exists() and not (part.is_file() if part == target else part.is_dir()):
            raise ConfigError('MCP configuration path is obstructed')


def _atomic(target, content, mode):
    descriptor, temporary_name = tempfile.mkstemp(prefix='.memory-mcp-', dir=target.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, 'wb') as handle:
            handle.write(content); handle.flush(); os.fsync(handle.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)


def apply(root, plans):
    """Commit a preflighted group; preserve external edits during rollback."""
    written, directories = [], []
    try:
        for target, previous, content in plans:
            if content == previous: continue
            _confined(root, target)
            if (target.read_bytes() if target.exists() else None) != previous:
                raise ConfigError('MCP configuration changed during registration')
            missing = []
            parent = target.parent
            while parent != root and not parent.exists(): missing.append(parent); parent = parent.parent
            for parent in reversed(missing): parent.mkdir(); directories.append(parent)
            mode = target.stat().st_mode & 0o777 if previous is not None else 0o600
            _atomic(target, content, mode)
            written.append((target, previous, content, mode))
    except (ConfigError, OSError):
        partial = False
        for target, previous, content, mode in reversed(written):
            try:
                _confined(root, target)
                if not target.is_file() or target.read_bytes() != content:
                    partial = True; continue
                if previous is None: target.unlink()
                else: _atomic(target, previous, mode)
            except (ConfigError, OSError): partial = True
        for directory in reversed(directories):
            try: directory.rmdir()
            except OSError: pass
        raise ConfigError('Memory MCP registration is partial; inspect client configs' if partial
            else 'Memory MCP registration failed and was rolled back')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--tool', action='append', choices=tuple(PATHS))
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    try:
        root = args.root.absolute()
        if root != root.resolve() or root != Path(__file__).resolve().parents[2]:
            raise ConfigError('Run the installed registration script in its own project')
        python = python_command()
        plans = []
        for host in dict.fromkeys(args.tool or PATHS):
            path = PATHS[host]; target = root / path
            _confined(root, target)
            previous = target.read_bytes() if target.exists() else None
            plans.append((target, previous, merge(path, previous, template(host), python)))
        if not args.dry_run: apply(root, plans)
        for target, previous, content in plans:
            if content == previous: continue
            print(('WOULD_REGISTER' if args.dry_run else 'REGISTERED') + '\t' + target.relative_to(root).as_posix())
    except (ValueError, OSError, UnicodeError) as error:
        print(str(error) if isinstance(error, ConfigError) else
            'Memory MCP registration failed. Inspect client configuration, Python and project paths before retrying.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__': raise SystemExit(main())
