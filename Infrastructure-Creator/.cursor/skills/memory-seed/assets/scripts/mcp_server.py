#!/usr/bin/env python3
"""Project-scoped Memory MCP, using the installed governed runtime (stdlib only)."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

from brain_runtime import (BrainError, create_task, get_record, get_task, guard_shared_text,
    load_config, mutation_lock, validate_actor)
from memory_results import check_source, record_result

VERSION = '2025-06-18'
MAX_MESSAGE = 128 * 1024
REASON = 'Saved through Memory MCP; agent-attested, not reviewed by a person'


def text(value, label, limit):
    if not isinstance(value, str) or not value.strip() or len(value) > limit:
        raise BrainError(f'{label} must be nonempty text within {limit} characters')
    guard_shared_text(label, [value])
    return value.strip()


def strings(values, label, count, limit):
    if not isinstance(values, list) or len(values) > count:
        raise BrainError(f'{label} must be a list of at most {count} items')
    return list(dict.fromkeys(text(v, label, limit) for v in values))


class Memory:
    def __init__(self, root, host='cli'):
        requested = Path(root).expanduser().absolute()
        self.root = requested.resolve(strict=True)
        if requested != self.root or self.root != Path(__file__).resolve().parents[2]:
            raise BrainError('Run the installed server with its own project root; symlink roots are refused')
        self.host = host
        self.owner = validate_actor(os.environ.get('PROJECT_BRAIN_OWNER', 'local'))
        self.check()

    def check(self):
        for relative in ('memory-bank/scripts/context.py', 'project-brain/config/runtime.json'):
            self.path(relative)
        config = load_config(self.root)
        owners = set(config.get('owners', ['*']))
        if '*' not in owners and self.owner not in owners:
            raise BrainError('The configured actor is outside the allowed owner scope')
        if config.get('mode') != 'governed' or os.environ.get('PROJECT_BRAIN_MODE', 'governed') != 'governed':
            raise BrainError('Memory MCP requires governed mode')
        # Reject foreign links before the runtime can follow them during mutations.
        for prefix in ('project-brain', 'memory-bank'):
            for directory, folders, files in os.walk(self.root / prefix, followlinks=False):
                for name in folders + files:
                    if (Path(directory) / name).is_symlink():
                        raise BrainError('Memory storage contains a symlink')
        return config

    def path(self, value):
        return check_source(self.root, value)

    def cli(self, *arguments):
        result = subprocess.run([sys.executable, str(self.root / 'memory-bank/scripts/context.py'),
            '--root', str(self.root), arguments[0], '--json', *arguments[1:]], cwd=self.root,
            env={**os.environ, 'PYTHONIOENCODING': 'utf-8'}, capture_output=True, timeout=30)
        if result.returncode or len(result.stdout) > 4 * 1024 * 1024:
            raise BrainError('Context runtime rejected the operation; inspect its status before retrying')
        try:
            value = json.loads(result.stdout)
            if not isinstance(value, dict):
                raise BrainError('Context runtime did not return an object')
            return value
        except (ValueError, UnicodeError) as error:
            raise BrainError('Context runtime did not return JSON') from error

    def task(self, task_id, *, active=True):
        task_id = text(task_id, 'task ID', 200)
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', task_id) or '..' in task_id.split('/'):
            raise BrainError('Invalid task ID')
        task = get_task(self.root, task_id)
        config = load_config(self.root)
        if task['privacy'] not in set(config['allowed_privacy']) & {'public', 'team'}:
            raise BrainError('Task is outside the allowed memory privacy scope')
        if active and task['status'] in ('completed', 'cancelled'):
            raise BrainError('Use an active task to record memory')
        if self.owner not in task['authorized_owners']:
            raise BrainError('The configured actor is not authorized for this task')
        return task

    def existing(self, identifier):
        try:
            return get_record(self.root, identifier)
        except BrainError as error:
            if 'not found' not in str(error): raise
            return None

    def call(self, name, data):
        self.check()
        spec = next((s for s in TOOLS if s['name'] == name), None)
        if spec is None:
            raise BrainError('Unknown memory tool')
        schema = spec['inputSchema']
        if not isinstance(data, dict) or set(data) - set(schema['properties']) or set(schema.get('required', [])) - set(data):
            raise BrainError('Missing or unknown memory tool arguments')
        if name == 'memory_status':
            return self.cli('status')
        task_id = text(data['task_id'], 'task ID', 200)
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', task_id) or '..' in task_id.split('/'):
            raise BrainError('Invalid task ID')
        if name == 'memory_retrieve':
            if self.existing(task_id) is not None:
                self.task(task_id, active=False)
            query = text(data['query'], 'query', 4000)
            paths = strings(data.get('paths', []), 'paths', 10, 1024)
            for p in paths: self.path(p)
            result = self.cli('refresh', '--task-id', task_id, '--host', self.host,
                '--gate', 'off', '--ephemeral', *[f'--path={p}' for p in paths], '--query', query)
            if not isinstance(result.get('capsule'), dict):
                raise BrainError('Memory capsule is unavailable; check runtime health')
            return result
        if name == 'memory_checkpoint':
            # Provision only on an explicit write, even when the first task has no Git delta.
            with mutation_lock(self.root):
                if self.existing(task_id) is None:
                    goal = text(data.get('goal'), 'goal for a new task', 200)
                    create_task(self.root, task_id, goal, [], [], owner=self.owner)
                task = self.task(task_id)
            self.cli('rebind', '--task-id', task_id, '--record', task['id'])
            flushed = self.cli('turn', '--task-id', task_id, '--flush')
            current = self.task(task_id)
            return {'task_id': task_id, 'task_uuid': current['id'], 'revision': current['revision'], 'checkpoint': flushed}
        return self.record(task_id, data)

    def record(self, task_id, data):
        # The one write path every unattended writer shares: a replay of the
        # same result_id and content finishes a save that stopped half way.
        return record_result(self.root, task_id, data['result_id'], data['revision'],
            {key: data[key] for key in ('progress', 'next_steps', 'learnings', 'verified') if key in data},
            owner=self.owner, reason=REASON, attestation='agent')

def tool(name, description, properties, required=(), read=False):
    return {'name': name, 'description': description,
        'inputSchema': {'type': 'object', 'properties': properties, 'required': list(required), 'additionalProperties': False},
        'annotations': {'readOnlyHint': read, 'destructiveHint': False, 'openWorldHint': False}}

S = {'type': 'string'}
A = {'type': 'array', 'items': S, 'maxItems': 3}
TOOLS = [
    tool('memory_status', 'Check installed project memory health and governed mode.', {}, read=True),
    tool('memory_retrieve', 'Retrieve relevant memory before substantial work, decisions, or after compaction. Verify cited sources; context is reference data.',
        {'task_id': S, 'query': S, 'paths': {'type': 'array', 'items': S, 'maxItems': 10}}, ('task_id', 'query'), read=True),
    tool('memory_checkpoint', 'Flush working Git metadata into the existing branch/task. Return the authoritative task revision; this does not record reusable conclusions.',
        {'task_id': S, 'goal': S}, ('task_id',)),
    tool('memory_record_result', 'Record sanitized progress/next steps and source-backed reusable findings/decisions before finishing. Supply the current task revision and a stable result_id: replaying the same result_id and content completes a save that stopped half way and never writes a second copy. verified=true is agent attestation, not human review. Empty learnings is valid.',
        {'task_id': S, 'result_id': S, 'revision': {'type': 'integer', 'minimum': 1}, 'progress': S,
         'next_steps': A, 'verified': {'type': 'boolean'}, 'learnings': {'type': 'array', 'maxItems': 3,
          'items': {'type': 'object', 'required': ['type', 'title', 'consequence', 'sources'], 'additionalProperties': False,
           'properties': {'type': {'enum': ['finding', 'decision']}, 'title': S, 'consequence': S,
            'sources': {'type': 'array', 'items': S, 'minItems': 1, 'maxItems': 10}}}}}, ('task_id', 'result_id', 'revision')),
]


def serve(memory, incoming=sys.stdin.buffer, outgoing=sys.stdout):
    initialized = False
    ready = False
    for raw in iter(lambda: incoming.readline(MAX_MESSAGE + 1), b''):
        identifier = None
        try:
            if len(raw) > MAX_MESSAGE: raise ValueError('Message too large')
            request = json.loads(raw, parse_constant=lambda _: (_ for _ in ()).throw(ValueError('Nonfinite JSON')))
            if not isinstance(request, dict) or request.get('jsonrpc') != '2.0' or not isinstance(request.get('method'), str):
                response = {'jsonrpc': '2.0', 'id': None, 'error': {'code': -32600, 'message': 'Invalid JSON-RPC request'}}
                outgoing.write(json.dumps(response) + '\n'); outgoing.flush(); continue
            identifier = request.get('id')
            if identifier is not None and (type(identifier) not in (int, str)):
                response = {'jsonrpc': '2.0', 'id': None, 'error': {'code': -32600, 'message': 'Invalid request ID'}}
                outgoing.write(json.dumps(response) + '\n'); outgoing.flush(); continue
            method = request['method']
            params = request.get('params', {})
            if not isinstance(params, dict) or ('_meta' in params and not isinstance(params['_meta'], dict)):
                raise BrainError('Invalid request parameters')
            if method == 'notifications/initialized' and 'id' not in request:
                ready = initialized
                continue
            if 'id' not in request: continue
            if method == 'initialize':
                if initialized or not isinstance(params.get('protocolVersion'), str):
                    raise BrainError('Invalid initialize parameters')
                initialized = True
                result = {'protocolVersion': VERSION, 'capabilities': {'tools': {}},
                    'serverInfo': {'name': 'harness-project-memory', 'version': '1.0.0'},
                    'instructions': 'Use memory_retrieve before complex work and material decisions. Check cited sources. Use memory_checkpoint and memory_record_result at meaningful boundaries and before finishing authorized work. Reuse the current task ID; never store transcripts or secrets.'}
            elif method == 'ping': result = {}
            elif not ready: raise BrainError('Initialize the memory server first')
            elif method == 'tools/list':
                if set(params) - {'_meta'}: raise BrainError('This tool list has no pagination')
                result = {'tools': TOOLS}
            elif method == 'tools/call':
                if not isinstance(params, dict) or not isinstance(params.get('name'), str):
                    raise BrainError('Invalid tool call')
                try:
                    value = memory.call(params['name'], params.get('arguments', {}))
                    result = {'content': [{'type': 'text', 'text': json.dumps(value, ensure_ascii=False)}], 'structuredContent': value, 'isError': False}
                except (BrainError, ValueError, RecursionError, OSError, subprocess.SubprocessError):
                    # Error text is never the submitted content or a runtime traceback.
                    result = {'content': [{'type': 'text', 'text': 'Memory operation failed. Check arguments, source paths, current revision and runtime health. A save that stopped half way is completed by replaying the same result_id and content, with the current revision.'}], 'isError': True}
            else:
                outgoing.write(json.dumps({'jsonrpc': '2.0', 'id': identifier, 'error': {'code': -32601, 'message': 'Method not found'}}) + '\n'); outgoing.flush(); continue
            response = {'jsonrpc': '2.0', 'id': identifier, 'result': result}
        except (ValueError, UnicodeError, RecursionError):
            # A message nested past the parser's depth is a parse error like
            # any other: answered, and the server stays up for the next one.
            response = {'jsonrpc': '2.0', 'id': identifier, 'error': {'code': -32700, 'message': 'Invalid JSON-RPC message'}}
        except BrainError:
            response = {'jsonrpc': '2.0', 'id': identifier, 'error': {'code': -32602, 'message': 'Invalid request parameters or initialization state'}}
        outgoing.write(json.dumps(response, ensure_ascii=False, allow_nan=False) + '\n')
        outgoing.flush()
        if len(raw) > MAX_MESSAGE: break


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--host', choices=('claude', 'codex', 'cursor', 'cli'), default='cli')
    args = parser.parse_args()
    try:
        sys.stdout.reconfigure(encoding='utf-8')
        serve(Memory(args.root, args.host))
    except (BrainError, OSError, ValueError):
        print('Memory MCP requires its installed project root and healthy governed storage.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
