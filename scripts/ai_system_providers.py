"""Trusted native CLI adapters for system workers (no third-party dependencies)."""
from __future__ import annotations

from pathlib import Path
import sys

# Share the Harness permission/delegation flags; never import target-project code.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness.providers import build_command, discover_providers, normalize_event
from ai_system_lib import SystemError, encoded, parse_json

NATIVE_PROVIDERS = ('codex', 'claude', 'cursor')


def discover_executable(provider):
    return next((p['executable'] for p in discover_providers()
                 if p['id'] == provider and p['available']), None)


def invocation(provider, executable, root, prompt, schema, schema_path, mode):
    if provider == 'command':
        return [executable], prompt
    if provider == 'cursor' and len(prompt.encode('utf-8')) > 120000:
        raise SystemError('Cursor prompt exceeds the argument byte limit; narrow the plan')
    command = build_command(provider, executable, root, prompt,
                            mode='edit' if mode == 'edit' else 'plan', agents_enabled=False)
    if provider == 'codex':
        command[-1:-1] = ['--skip-git-repo-check', '--output-schema', str(schema_path)]
    elif provider == 'claude':
        # Documented JSON Schema output is a single result envelope, not prose.
        command[command.index('--output-format') + 1] = 'json'
        command.remove('--verbose')
        command.extend(['--json-schema', encoded(schema)])
    return command, '' if provider == 'cursor' else prompt


def worker_result(raw, provider):
    if provider == 'command':
        return parse_json(raw)
    if provider not in NATIVE_PROVIDERS:
        raise SystemError('Unknown native worker provider')
    events = [parse_json(raw)] if provider == 'claude' else [parse_json(line) for line in raw.splitlines()]
    answer, terminal, failed = None, 0, False
    for event in events:
        if not isinstance(event, dict):
            raise SystemError('Invalid native worker event')
        normalized = normalize_event(provider, event)
        failed |= any(e['kind'] == 'error' for e in normalized)
        for item in normalized:
            if item['kind'] == 'result':
                terminal += 1
                failed |= not item['ok']
                if provider == 'claude':
                    answer = event.get('structured_output')
                elif provider == 'cursor':
                    answer = event.get('result')
            elif provider == 'codex' and item['kind'] == 'text':
                if terminal:
                    failed = True
                answer = item['text']
    if terminal != 1 or failed:
        raise SystemError('Worker did not return a successful terminal result')
    if provider == 'claude':
        if not isinstance(answer, dict):
            raise SystemError('Claude worker did not return structured output')
        return answer
    if not isinstance(answer, str) or not answer:
        raise SystemError('Worker did not return a JSON report')
    return parse_json(answer.encode('utf-8'))
