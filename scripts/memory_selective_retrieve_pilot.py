#!/usr/bin/env python3
"""Experimental one-hook recall/full router. Never installed by default."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import time

DEFAULT_RULES = {
    'version': 2,
    'historical': ['раньше', 'ранее', 'прежде', 'в прошлом', 'до изменения',
                   'previous', 'previously', 'earlier', 'former', 'formerly',
                   'before', 'used to', 'old', 'as of'],
    'decisions': ['отказались', 'решили', 'выбрали', 'приняли решение',
                  'rejected', 'decided', 'chose', 'did we choose', 'did we reject'],
    'inquiries': ['какой', 'какая', 'какое', 'какие', 'что', 'как', 'почему', 'зачем',
                  'когда', 'найди', 'вспомни', 'what', 'which', 'how', 'why', 'when',
                  'recover', 'recall', 'find', 'explain'],
    'current': ['сейчас', 'теперь', 'текущий', 'актуальный', 'current', 'currently', 'today'],
    'procedural': ['как выполнять', 'порядок действий', 'процедура', 'инструкция',
                   'how to', 'workflow', 'procedure'],
    'continuations': ['дальше', 'продолжай', 'продолжи', 'делай', 'сделай',
                       'continue', 'go ahead'],
}
ROUTE_REASONS = {'historical-state', 'historical-decision', 'current-state',
                 'procedural', 'continuation', 'ordinary'}
MAX_PAYLOAD = 1024 * 1024


def canonical_json(value) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False,
                      allow_nan=False).encode('utf-8')


def choose_route(query: str, rules: dict) -> tuple[str, str]:
    if (not isinstance(query, str) or len(query) > 24000 or not isinstance(rules, dict)
            or type(rules.get('version')) is not int or rules['version'] not in (1, 2)):
        raise ValueError('invalid routing rules')
    groups = ({'historical', 'decisions'} if rules['version'] == 1 else
              {'historical', 'decisions', 'inquiries', 'current', 'procedural', 'continuations'})
    if set(rules) != groups | {'version'}:
        raise ValueError('invalid routing rules')
    for name in groups:
        values = rules[name]
        if (not isinstance(values, list) or not 1 <= len(values) <= 64
                or any(not isinstance(v, str) or not v.strip() or len(v) > 80
                       or not re.fullmatch(r'[\w -]+', v, re.UNICODE) for v in values)):
            raise ValueError('invalid bounded phrase list')
    folded = ' ' + ' '.join(re.findall(r'\w+', query.casefold())) + ' '
    matches = lambda name: any(' ' + ' '.join(re.findall(r'\w+', value.casefold())) + ' ' in folded
                               for value in rules[name])
    if rules['version'] == 1:
        historical = matches('historical') and matches('decisions')
        return ('full', 'historical-decision') if historical else ('recall', 'ordinary')
    if any(folded == ' ' + ' '.join(re.findall(r'\w+', value.casefold())) + ' '
           for value in rules['continuations']):
        return 'recall', 'continuation'
    inquiry = '?' in query or any(folded.startswith(' ' + ' '.join(
        re.findall(r'\w+', value.casefold())) + ' ') for value in rules['inquiries'])
    if inquiry and (matches('historical') or matches('decisions')):
        return 'full', 'historical-decision' if matches('decisions') else 'historical-state'
    if matches('procedural'):
        return 'recall', 'procedural'
    if inquiry and matches('current'):
        return 'recall', 'current-state'
    return 'recall', 'ordinary'


def choose_profile(query: str, rules: dict) -> str:
    return choose_route(query, rules)[0]


def append_audit(path: Path, record: dict) -> bool:
    try:
        if any(parent.is_symlink() for parent in [path, *path.parents]): return False
        fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT | getattr(os, 'O_NOFOLLOW', 0)
                     | getattr(os, 'O_NONBLOCK', 0), 0o600)
        try:
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_PAYLOAD: return False
            os.fchmod(fd, 0o600) if hasattr(os, 'fchmod') else None
            os.write(fd, canonical_json(record) + b'\n')
        finally: os.close(fd)
        return True
    except (OSError, ValueError): return False


def route_payload(raw: bytes, rules_path: Path, runtime_dir: Path) -> tuple[str, dict]:
    started = time.monotonic_ns()
    record = {'schema': 'memory-selective-route-v2', 'profile': 'recall', 'reason': 'routing-unavailable',
              'rules_digest': None, 'router_digest': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'sanitizer_digest': None}
    try:
        if len(raw) > MAX_PAYLOAD: raise ValueError('oversized payload')
        data = json.loads(raw.decode('utf-8'))
        prompt = data.get('prompt')
        if not isinstance(prompt, str): raise ValueError('prompt unavailable')
        if len(prompt) > 24000: prompt = prompt[:15999] + '\n' + prompt[-8000:]
        if rules_path.stat().st_size > 16384: raise ValueError('oversized rules')
        rules = json.loads(rules_path.read_text(encoding='utf-8'))
        sanitizer = runtime_dir / 'automatic_query.py'
        record['sanitizer_digest'] = hashlib.sha256(sanitizer.read_bytes()).hexdigest()
        spec = importlib.util.spec_from_file_location('selective_query_sanitizer', sanitizer)
        module = importlib.util.module_from_spec(spec)
        old = sys.dont_write_bytecode; sys.dont_write_bytecode = True
        try: spec.loader.exec_module(module)
        finally: sys.dont_write_bytecode = old
        query = module.sanitize_automatic_query(prompt)
        record['profile'], record['reason'] = choose_route(query, rules)
        record['rules_digest'] = hashlib.sha256(canonical_json(rules)).hexdigest()
    except (OSError, ValueError, TypeError, AttributeError, ImportError, TimeoutError, RecursionError):
        pass
    record['routing_ns'] = time.monotonic_ns() - started
    return record['profile'], record


def parse_observations(raw: bytes, expected: dict) -> dict:
    unknown = {'status': 'unknown', 'records': [], 'producer_authenticated': False}
    if len(raw) > MAX_PAYLOAD: return unknown
    try:
        records = [json.loads(line) for line in raw.decode('utf-8').splitlines() if line]
        keys = {'schema', 'profile', 'reason', 'rules_digest', 'router_digest', 'sanitizer_digest', 'routing_ns', 'hook_exit'}
        if not records or len(records) > 1000: return unknown
        for row in records:
            if (not isinstance(row, dict) or set(row) != keys
                    or row['schema'] not in ('memory-selective-route-v1', 'memory-selective-route-v2')
                    or row['profile'] not in ('recall', 'full')
                    or row['reason'] not in ROUTE_REASONS | {'routing-unavailable'}
                    or type(row['routing_ns']) is not int or row['routing_ns'] < 0
                    or type(row['hook_exit']) is not int): return unknown
            if row['schema'] == 'memory-selective-route-v1' and row['reason'] not in (
                    'routing-unavailable', 'historical-decision', 'ordinary'):
                return unknown
            if ((row['reason'].startswith('historical-') and row['profile'] != 'full')
                    or (not row['reason'].startswith('historical-') and row['profile'] != 'recall')):
                return unknown
            for key, value in expected.items():
                if row.get(key) != value and not (row['reason'] == 'routing-unavailable'
                                                  and key != 'router_digest' and row.get(key) is None):
                    return unknown
        return {'status': 'observed', 'records': records, 'producer_authenticated': False}
    except (ValueError, TypeError, UnicodeError, RecursionError): return unknown


def wrap_hook_command(command: str, *, script: Path, rules_path: Path,
                      audit_path: Path, runtime_dir: Path) -> str:
    import shlex
    return shlex.join([sys.executable, '-B', str(script), '--enabled', '--rules', str(rules_path),
                      '--audit', str(audit_path), '--runtime-dir', str(runtime_dir), '--hook', command])


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--hook', required=True)
    parser.add_argument('--enabled', action='store_true')
    parser.add_argument('--rules', type=Path)
    parser.add_argument('--audit', type=Path)
    parser.add_argument('--runtime-dir', type=Path)
    args = parser.parse_args(argv)
    command = [shutil.which('bash') or '/bin/sh', '-c', args.hook]
    if not args.enabled:
        os.execvpe(command[0], command, os.environ.copy())
    raw = sys.stdin.buffer.read(MAX_PAYLOAD + 1)
    profile, record = route_payload(raw, args.rules, args.runtime_dir)
    environment = {**os.environ, 'CONTEXT_CAPSULE_PROFILE': profile}
    # Oversized frames still reach the original hook byte-for-byte, once,
    # without an unbounded in-memory copy in the router.
    if len(raw) > MAX_PAYLOAD:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, env=environment)
        try:
            process.stdin.write(raw)
            shutil.copyfileobj(sys.stdin.buffer, process.stdin, 65536)
            process.stdin.close()
        except BrokenPipeError: pass
        code = process.wait()
    else:
        code = subprocess.run(command, input=raw, env=environment).returncode
    record['hook_exit'] = code
    if args.audit is not None: append_audit(args.audit, record)
    return code


if __name__ == '__main__': raise SystemExit(main())
