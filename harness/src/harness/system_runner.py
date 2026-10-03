"""Trusted queue worker for reviewed system plans; no browser-supplied command."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import signal
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
sys.path.insert(0, str(ROOT / 'harness/src'))
import ai_system_execution as execution
from ai_system_lib import System, SystemError
from harness.agent_activity import ActivityStream

LABELS = {'contracts': 'Contract agent', 'verify': 'Verification agent'}


def emit(kind, text, **extra):
    print(json.dumps({'kind': kind, 'text': text[:4000], **extra}), flush=True)


def send(event):
    print(json.dumps(event, ensure_ascii=False), flush=True)


def observer_for(state):
    """Forward each dispatch's lifecycle and live native activity to the session."""
    stream = ActivityStream(send, state['provider'], state['roots'])

    def observe(event):
        kind = event['type']
        if kind == 'output':
            stream.line(event['line'])
        elif kind == 'dispatch_started':
            phase = event['phase']
            stream.start(phase, event['attempt'], event['root'], service=event['service'],
                         label=LABELS.get(phase, event['service'] + ' agent'), mode=event['mode'],
                         provider=event['provider'], root=event['root'], dispatch=event['dispatch_id'],
                         readable=event['readable'], writable=event['writable'])
        elif kind == 'dispatch_finished':
            report = event.get('report') or {}
            checks = report.get('checks') or []
            status = 'interrupted' if event.get('interrupted') else 'completed' if event['ok'] else 'blocked'
            stream.finish(status, ok=event['ok'], error=event['error'],
                          duration_seconds=event.get('duration_seconds'), summary=report.get('summary'),
                          changed_files=(report.get('changed_files') or [])[:100],
                          checks={name: sum(c['status'] == name for c in checks) for name in ('passed', 'failed', 'not_run')})
    observe.stream = stream
    return observe


def interrupted(*_):
    raise KeyboardInterrupt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--request', type=Path, required=True)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    execution.PROCESS_GUARD = Path(__file__).with_name('process_guard.py')
    try:
        request = execution.load(args.request.parent, args.request.name)
        directory = Path(request['directory'])
        if directory != args.request.parent:
            raise SystemError('System request directory changed')
        system = System(request['system_file'], request['allowed_roots'])
        provider = request.get('provider', 'codex')  # Legacy requests used Codex.
        if provider not in execution.NATIVE_PROVIDERS:
            raise SystemError('Invalid system worker provider')
        journal = directory / 'execution'
        if (journal / 'run.json').exists():
            state = execution.validate_state(system, journal)
            if (state['provider'] != provider or
                    state['executable'] != execution.executable_path(provider, request['executable'])):
                raise SystemError('Configured provider changed; prepare a new plan')
        else:
            plan = execution.load(directory, 'approved-plan.json')
            state = execution.create_run(system, plan, journal, provider, request['executable'],
                                         request['mode'], request['timeout'], request.get('access', 'service'))
        emit('status', 'Sequential system dispatch started. Progress and receipts are saved after each dispatch.')
        observe = observer_for(state)
        state = execution.drive_run(system, journal, state, request['retry_step'], request['accept_source_changes'],
                                    observer=observe)
        ok = state['status'] == 'completed'
        stopped = next((s for s in state['steps'] if s['status'] in ('blocked', 'interrupted')), None)
        reason = stopped and observe.stream.reasons.get(stopped['id'])
        if reason:
            # The journal keeps a generic code; the CLI's own error says what to fix.
            emit('error', LABELS.get(stopped['id'], stopped['service'] + ' agent') + ': ' + reason)
        emit('text', state['error'] or 'Worker-reported checks completed; native tasks closed and handoff saved.')
        emit('result', 'System run ' + state['status'], ok=ok)
        return 0 if ok else 1
    except KeyboardInterrupt:
        emit('result', 'System run interrupted; inspect saved dispatches before recovery.', ok=False)
        return 1
    except (SystemError, OSError, ValueError, KeyError) as error:
        emit('error', str(error) if isinstance(error, SystemError) else 'System runner could not read its saved inputs.')
        emit('result', 'System run blocked. Inspect the plan and execution journal.', ok=False)
        return 1


if __name__ == '__main__':
    sys.exit(main())
