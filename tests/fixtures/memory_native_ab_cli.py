"""Offline public adapter fixture. Executes real hooks; never contacts a model."""
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import uuid
import sys

root = Path.cwd()
task = os.environ['CONTEXT_TASK_ID']
session = str(uuid.uuid4())
source = root.name == 'source'
hooks = json.loads((root / '.codex/hooks.json').read_text())['hooks']
answer = 'The Amber dispatcher retries only idempotent requests.'


def emit(value):
    print(json.dumps(value), flush=True)


def hook(event, **extra):
    responses = []
    payload = dict(hook_event_name=event, session_id=session, cwd=str(root), **extra)
    for group in hooks[event]:
        for handler in group['hooks']:
            run = subprocess.run(['sh', '-c', handler['command']], input=json.dumps(payload).encode(),
                                 capture_output=True, timeout=20)
            if run.returncode:
                raise RuntimeError('fixture hook failed: ' + str(run.returncode) + run.stderr.decode()[:500])
            try:
                responses.append(json.loads(run.stdout))
            except ValueError:
                pass
    return responses


def tool(identity, script):
    run = subprocess.run(['bash', '-lc', script], capture_output=True, text=True, timeout=20)
    emit({'type': 'item.completed', 'item': {'id': identity, 'type': 'command_execution',
          'command': '/usr/bin/bash -lc ' + shlex.quote(script),
          'aggregated_output': run.stdout, 'exit_code': run.returncode}})
    if run.returncode:
        raise RuntimeError('fixture tool failed: ' + run.stderr[:500])
    return run.stdout


emit({'type': 'thread.started', 'thread_id': session})
emit({'type': 'turn.started'})
hook('SessionStart', source='startup')
hook('UserPromptSubmit', prompt='Inspect the public Amber retry rule.')
if not source:
    tool('unrelated-heredoc',
         "python3 - <<'PY'\n# don't interpret Python comments as shell quoting\nprint('ok')\nPY")
    recalled = json.loads(tool('own-recall', shlex.join(['python3', 'memory-bank/scripts/context.py',
        'refresh', '--query', 'Amber retry boundary', '--task-id', task,
        '--capsule-profile', 'recall', '--ephemeral', '--json'])))
    assert 'Amber retry boundary' not in recalled.get('capsule_text', '')
    tool('retrieve', shlex.join(['python3', 'memory-bank/scripts/context.py', 'retrieve',
        'Amber retry boundary', '--task-id', task, '--ephemeral']))
    chunks = sorted((root / 'memory-bank/chunks').glob('MEM-*.md'))
    memory = json.loads(chunks[0].read_text().split('---', 2)[1])['id']
    answer += ' Rechecked ' + memory + ' against rules.md.'
tool('source-read', 'cat rules.md')
emit({'type': 'item.completed', 'item': {'id': 'answer', 'type': 'agent_message', 'text': answer}})
stops = hook('Stop', last_assistant_message=answer, stop_hook_active=False)
reason = next(r['reason'] for r in stops if r.get('decision') == 'block')
request = re.search(r'--request ([0-9a-f]{64})', reason).group(1)
token = re.search(r'--token ([0-9a-f]{32})', reason).group(1)
payload = {'progress': 'Checked the public Amber retry rule.', 'verified': True, 'learnings': []}
if source:
    payload['learnings'] = [{'type': 'finding', 'title': 'Amber retry boundary',
        'consequence': 'The Amber dispatcher retries only idempotent requests.', 'sources': ['rules.md']}]
script = ('python3 memory-bank/scripts/context.py native-result --request ' + request +
          ' --token ' + token + " --input - --json <<'NATIVE_RESULT'\n" +
          json.dumps(payload) + '\nNATIVE_RESULT')
tool('save', script)
hook('Stop', last_assistant_message=answer, stop_hook_active=True)
emit({'type': 'item.completed', 'item': {'id': 'final', 'type': 'agent_message', 'text': answer}})
emit({'type': 'turn.completed', 'usage': {'input_tokens': 10, 'output_tokens': 2}})
if '--extra-public-file' in sys.argv:
    (root / 'unexpected-public.md').write_text('fixture-only unexpected data')
