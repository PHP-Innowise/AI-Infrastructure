"""One read-only, budgeted attempt to format a completed run's missing memory draft."""
from __future__ import annotations

import json
import os
import selectors
import subprocess
import threading
import time

from . import memory_draft, process_runtime, providers
from .filesystem import fs

TIME_LIMIT = 30
PROMPT = (
    'The preceding work has finished. This internal request only repairs its missing memory draft. '
    'Do not repeat the user task, edit files, run tests, delegate, or write memory yourself. '
    'Use only verified facts from this native session; leave learnings empty when none are supported. '
    'Return only the fenced memory-draft block. ' + memory_draft.instruction()
)


def recover(store, sid, session, generation, project, environment, *, remaining_seconds, exhausted, observe,
            observe_raw=None, on_launch=None, observe_transport=None, observe_boundary=None):
    launch = {'started': False}
    try:
        result = _recover(store, sid, session, generation, project, environment,
            remaining_seconds=remaining_seconds, exhausted=exhausted, observe=observe, launch=launch,
            observe_raw=observe_raw, on_launch=on_launch, observe_transport=observe_transport,
            observe_boundary=observe_boundary)
    except Exception:
        try:
            if observe_boundary is not None: observe_boundary('lifecycle_exception', None)
        except Exception: pass
        result = {'state': 'failed', 'reason': 'Memory recovery could not start; the completed work is preserved.'}
    return {**result, 'launched': launch['started']}


def _recover(store, sid, session, generation, project, environment, *, remaining_seconds, exhausted, observe, launch,
             observe_raw=None, on_launch=None, observe_transport=None, observe_boundary=None):
    def measured(callback, *args):
        try:
            if callback is not None:
                callback(*args)
        except Exception:
            pass
    def boundary(signal, code=None):
        measured(observe_boundary, signal, code)
    with store.lock:
        if sid in store.cancelled or store.stopping.is_set() or store.generations.get(sid) != generation:
            boundary('cancelled' if sid in store.cancelled else 'shutdown' if store.stopping.is_set() else 'generation_changed')
            return {'state': 'cancelled', 'reason': 'Memory recovery was cancelled before launch.'}
    native_id = session.get('native_session_id')
    if not native_id:
        boundary('recovery_skipped')
        return {'state': 'skipped', 'reason': 'The provider returned no resumable session identity.'}
    if exhausted or remaining_seconds is not None and remaining_seconds <= 0:
        boundary('recovery_skipped')
        return {'state': 'skipped', 'reason': 'The launch has no remaining budget.'}
    for cap, field in (('tokens', 'tokens'), ('usd', 'cost_usd')):
        if session['budgets'][cap] is not None and (session.get('budget_usage') or {}).get(field) is None:
            boundary('recovery_skipped')
            return {'state': 'skipped', 'reason': 'The remaining launch budget could not be measured.'}
    seconds = min(TIME_LIMIT, remaining_seconds) if remaining_seconds is not None else TIME_LIMIT
    budget_usd = session['budgets']['usd']
    if budget_usd is not None:
        budget_usd -= (session.get('budget_usage') or {}).get('cost_usd') or 0
        if budget_usd < .01:
            boundary('recovery_skipped')
            return {'state': 'skipped', 'reason': 'The remaining monetary budget is below the native minimum.'}
    command = providers.build_command(session['provider'], store.providers[session['provider']]['executable'],
        project, PROMPT, mode='plan', model=session['model'], session_id=native_id,
        agents_enabled=False, thinking_effort=None,
        **({'budget_usd': budget_usd} if budget_usd is not None else {}))
    env = {**environment, **providers.agent_environment(session['provider'], False, 3),
        'CONTEXT_MEMORY_RECOVERY': '1'}
    env.pop('CONTEXT_TASK_ID', None)
    env.pop('CONTEXT_CAPSULE_DELIVERED', None)
    process = None
    writer = None
    selector = process_runtime.PipeSelector()
    read_fd, write_fd = os.pipe()
    replies, terminal, failed, buffer, size = [], False, False, b'', 0
    try:
        with store.lock:
            if sid in store.cancelled or store.stopping.is_set() or store.generations.get(sid) != generation:
                boundary('cancelled' if sid in store.cancelled else 'shutdown' if store.stopping.is_set() else 'generation_changed')
                return {'state': 'cancelled', 'reason': 'Memory recovery was cancelled before launch.'}
            try:
                process = process_runtime.launch_guarded(command, read_fd, lock_fd=store.runner_lock,
                    cwd=project, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            except Exception:
                boundary('launch_exception')
                raise
            store.active[sid] = process
            launch['started'] = True
            measured(on_launch)
        fs.close(read_fd); read_fd = None
        selector.register(process.stdout, selectors.EVENT_READ)
        def write():
            try:
                value = providers.input_text(session['provider'], PROMPT)
                if value is not None:
                    count = process.stdin.write(value.encode('utf-8')); process.stdin.flush()
                    measured(observe_transport, count, 'written')
                else:
                    measured(observe_transport, None, 'not_used')
            except (OSError, ValueError):
                measured(observe_transport, None, 'failed')
                boundary('stdin_write_failed')
            finally:
                try: process.stdin.close()
                except OSError: pass
        writer = threading.Thread(target=write, daemon=True); writer.start()
        started = time.monotonic()
        while True:
            if sid in store.cancelled or store.stopping.is_set() or store.generations.get(sid) != generation:
                boundary('cancelled' if sid in store.cancelled else 'shutdown' if store.stopping.is_set() else 'generation_changed')
                return {'state': 'cancelled', 'reason': 'Memory recovery was cancelled; no draft was saved.'}
            if time.monotonic() - started > seconds:
                boundary('time_budget')
                return {'state': 'failed', 'reason': 'Memory recovery reached its bounded time limit.'}
            ready = selector.select(.1)
            if not ready:
                if process.poll() is not None and not process_runtime.WINDOWS: break
                continue
            chunk = selector.read(process.stdout, 65536)
            if not chunk: boundary('empty_read_ended_or_error')
            buffer += chunk if chunk else b'\n'
            size += len(chunk)
            if size > 512 * 1024:
                boundary('output_total_limit')
                return {'state': 'failed', 'reason': 'Memory recovery exceeded its output limit.'}
            lines = buffer.split(b'\n'); buffer = lines.pop()
            for line in lines:
                try: event = json.loads(line)
                except (ValueError, UnicodeError):
                    if line.strip(): boundary('malformed_line')
                    continue
                if not isinstance(event, dict):
                    boundary('non_object_line')
                    continue
                measured(observe_raw, event)
                for clean in providers.normalize_event(session['provider'], event, targets=True):
                    if clean.get('native_session_id') and clean['native_session_id'] != native_id:
                        boundary('identity_changed')
                        return {'state': 'failed', 'reason': 'Memory recovery changed the native session identity.'}
                    if clean.get('kind') == 'result':
                        terminal = True; failed |= clean.get('ok') is not True
                    if clean.get('kind') in ('text', 'result') and isinstance(clean.get('text'), str):
                        replies.append(clean['text'][:memory_draft.TEXT_LIMIT])
                    if observe(event, clean):
                        # The callback proves a shared-budget stop, not which cap.
                        boundary('shared_budget')
                        return {'state': 'failed', 'reason': 'Memory recovery reached the remaining launch budget.'}
            if not chunk: break
        if sid in store.cancelled or store.stopping.is_set() or store.generations.get(sid) != generation:
            boundary('cancelled' if sid in store.cancelled else 'shutdown' if store.stopping.is_set() else 'generation_changed')
            return {'state': 'cancelled', 'reason': 'Memory recovery was cancelled; no draft was saved.'}
        try:
            code = process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            boundary('wait_timeout')
            raise
        boundary('guard_exit_before_cleanup', code)
        if not terminal: boundary('missing_terminal')
        if code != 0 or not terminal or failed:
            return {'state': 'failed', 'reason': 'The provider did not complete memory recovery successfully.'}
        for reply in reversed(replies):
            if memory_draft.parse(reply) is not None:
                return {'state': 'recovered', 'reply': reply}
        boundary('recovery_no_draft')
        return {'state': 'failed', 'reason': 'The single recovery attempt returned no valid memory draft.'}
    except Exception:
        boundary('lifecycle_exception')
        return {'state': 'failed', 'reason': 'Memory recovery could not finish; the completed work is preserved.'}
    finally:
        if read_fd is not None: fs.close(read_fd)
        fs.close(write_fd)
        if process is not None:
            reaped = process_runtime.reap_tree(process)
            boundary('cleanup_confirmed' if reaped else 'cleanup_unconfirmed')
            boundary('guard_exit_after_cleanup', process.returncode)
            if not reaped: store.stopping.set()
            process.stdout.close()
            if writer is not None: writer.join(timeout=1)
            with store.lock:
                if reaped: store.active.pop(sid, None)
        selector.close()
