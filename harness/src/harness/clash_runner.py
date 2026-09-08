"""Guarded clash cycle: two native providers take turns; the ledger is emitted after each turn.

Invoked by the stdlib session runner under its process guard, standard
library only. The request arrives on stdin as JSON; events leave on stdout as
JSON lines. The outer guard owns the process group and the shared time
budget, so a lost owner or cancellation stops every participant.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import selectors
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from harness import clash, providers
from harness.sessions import SessionError, run_git

TOOL_EVENT_LIMIT = 300
TEXT_LIMIT = 32000
OUTPUT_LIMIT = 4 * 1024 * 1024
REQUEST_LIMIT = 1024 * 1024


def emit(event):
    print(json.dumps(event, ensure_ascii=False, allow_nan=False), flush=True)


def workspace_diff(project, base):
    """Bounded diff against the recorded launch baseline, plus untracked names."""
    if not isinstance(base, str) or not base:
        return None
    try:
        patch = run_git(project, 'diff', '--no-ext-diff', '--no-textconv', '--relative', '--no-renames', base, '--', '.')
        others = run_git(project, 'ls-files', '--others', '--exclude-standard', '--', '.')
    except SessionError:
        return None
    if patch.returncode or others.returncode:
        return None
    text = patch.stdout
    untracked = [line for line in others.stdout.splitlines() if line]
    if untracked:
        text += '\nUntracked files (not in the diff above; read them in the workspace): ' + ', '.join(untracked[:200]) + '\n'
    return {'text': text[:clash.DIFF_LIMIT], 'truncated': len(text) > clash.DIFF_LIMIT}


class Turn:
    def __init__(self):
        self.ok = False
        self.text = ''
        self.native_id = None
        self.cost = None
        self.seconds = 0.0
        self.error = None
        self.limit_reached = None


def run_turn(command, cwd, stdin_text, provider, tags, environment):
    """Stream one native run, forwarding public events tagged with the clash role."""
    turn = Turn()
    started = time.monotonic()
    process = subprocess.Popen(command, cwd=cwd, env=environment, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)

    def feed():
        try:
            if stdin_text is not None:
                process.stdin.write(stdin_text.encode('utf-8'))
                process.stdin.flush()
        except (BrokenPipeError, OSError):
            pass
        finally:
            try:
                process.stdin.close()
            except OSError:
                pass

    writer = threading.Thread(target=feed, daemon=True)
    writer.start()
    buffer, total, terminal, failed, tools = b'', 0, False, False, 0
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        while True:
            if not selector.select(.2):
                if process.poll() is not None:
                    break
                continue
            chunk = os.read(process.stdout.fileno(), 65536)
            buffer += chunk or b'\n'
            total += len(chunk)
            if len(buffer) > 2 * 1024 * 1024 or total > OUTPUT_LIMIT:
                raise SessionError('Participant output limit reached.')
            lines = buffer.split(b'\n')
            buffer = lines.pop()
            for line in lines:
                if not line:
                    continue
                try:
                    event = json.loads(line)
                except (ValueError, UnicodeError):
                    continue
                if not isinstance(event, dict):
                    continue
                if provider == 'claude' and event.get('subtype') == 'error_max_budget_usd':
                    turn.limit_reached = 'USD'
                for item in providers.normalize_event(provider, event):
                    if item.get('native_session_id'):
                        turn.native_id = item['native_session_id']
                    kind = item['kind']
                    if kind == 'session':
                        continue
                    if kind == 'result':
                        terminal = True
                        failed = failed or item.get('ok') is not True
                        if item.get('ok') and item.get('text'):
                            turn.text = item['text'][:TEXT_LIMIT]
                        continue
                    if kind == 'text':
                        turn.text = item['text'][:TEXT_LIMIT]
                    elif kind == 'usage':
                        if 'cost_usd' in item:
                            turn.cost = item['cost_usd']
                    elif kind == 'tool':
                        tools += 1
                        if tools > TOOL_EVENT_LIMIT:
                            if tools == TOOL_EVENT_LIMIT + 1:
                                emit({'kind': 'status', 'text': 'Further tool activity for this turn is not shown.', **tags})
                            continue
                    emit({**item, **tags})
            if not chunk:
                break
        code = process.wait(timeout=3)
        turn.ok = code == 0 and terminal and not failed
        if turn.limit_reached == 'USD':
            turn.ok = False
            turn.error = 'The participant stopped at its native USD budget.'
        elif not turn.ok:
            turn.error = 'The native participant did not complete successfully.'
    except (OSError, SessionError, subprocess.TimeoutExpired) as error:
        turn.ok = False
        turn.error = str(error) if isinstance(error, SessionError) else 'The native participant could not be read.'
    finally:
        selector.close()
        process.stdout.close()
        if process.poll() is None:
            process.kill()
            process.wait()
        writer.join(timeout=1)
        turn.seconds = round(time.monotonic() - started, 3)
    return turn


class Cycle:
    def __init__(self, request):
        self.request = request
        session = request['session']
        self.session = session
        self.settings = session['clash']
        self.project = Path(session['project_path'])
        self.previous = session.get('clash_result') if request['action'] == 'continue' else None
        self.state = clash.new_state(session, request['stage'], self.previous)
        self.native = {'protagonist': session.get('native_session_id'), 'challenger': self.state['challenger_session_id']}
        self.spent = 0.0
        self.cap = (session.get('budgets') or {}).get('usd')

    def emit_state(self):
        self.state['challenger_session_id'] = self.native['challenger']
        emit(self.state)

    def allowance(self, provider):
        if self.cap is None or provider != 'claude':
            return {}
        remaining = round(self.cap - self.spent, 6)
        if remaining < .01:
            raise SessionError('The USD budget is exhausted before this turn; raise the budget and send a follow-up.')
        return {'budget_usd': min(remaining, 1000)}

    def turn(self, role, prompt, mode, round_):
        participant = self.state['challenger'] if role == 'challenger' else self.state['protagonist']
        slot = 'challenger' if role == 'challenger' else 'protagonist'
        provider = participant['provider']
        executable = self.request['executables'].get(provider)
        if not executable:
            raise SessionError(f'The {provider} executable is unavailable for the {role} turn.')
        tags = {'role': role, 'provider': provider, 'round': round_, 'cycle': self.state['cycle']}
        prompt = prompt + '\n\n' + self.request.get('context', '')
        command = providers.build_command(provider, executable, self.project, prompt, mode=mode, model=participant.get('model'),
                                          session_id=self.native[slot], agents_enabled=False, agent_count=1,
                                          thinking_effort=participant.get('thinking_effort'), **self.allowance(provider))
        if provider == 'claude' and self.request.get('attachment_dirs'):
            command.extend(['--add-dir', *self.request['attachment_dirs']])
        environment = {**os.environ, **providers.agent_environment(provider, False, 1)}
        label = f"Cycle {self.state['cycle']}{f' · round {round_}' if round_ else ''} · {clash._name(provider)} ({role})"
        emit({'kind': 'clash_turn', 'status': 'running', 'text': label + ' is running.', **tags})
        result = run_turn(command, self.project, providers.input_text(provider, prompt), provider, tags, environment)
        if result.native_id:
            if self.native[slot] and self.native[slot] != result.native_id:
                result.ok = False
                result.error = 'The participant changed its native session identity.'
            elif not self.native[slot]:
                self.native[slot] = result.native_id
                if slot == 'protagonist':
                    emit({'kind': 'session', 'native_session_id': result.native_id})
        if isinstance(result.cost, (int, float)) and math.isfinite(result.cost) and result.cost >= 0:
            self.spent += result.cost
        clash.record_turn(self.state, {'cycle': self.state['cycle'], 'round': round_, 'role': role, 'provider': provider,
                                       'ok': result.ok, 'summary': clash.prose(result.text), 'cost_usd': result.cost,
                                       'seconds': result.seconds, 'error': result.error})
        emit({'kind': 'clash_turn', 'status': 'completed' if result.ok else 'failed',
              'text': label + (' completed.' if result.ok else f" failed: {result.error}"), **tags})
        return result

    def fail(self, message):
        self.state['status'] = 'finished'
        self.state['outcome'] = 'incomplete'
        self.state['message'] = message
        self.state['report'] = clash.render_report(self.state)
        self.state['message'] = clash.outcome_message(self.state)
        self.emit_state()
        emit({'kind': 'error', 'text': message})
        emit({'kind': 'result', 'ok': False, 'text': ''})

    def run(self):
        state, settings = self.state, self.settings
        stage = state['stage']
        protagonist_role = state['protagonist']['role']
        mode = 'edit' if stage == 'implement' else 'plan'
        prompt_text = self.request['prompt']
        started = any(turn.get('ok') and turn.get('role') != 'challenger' for turn in state['turns'])
        if self.request['action'] == 'start' or not started or not state['task']:
            # A cycle whose opening never finished has nothing to continue from.
            state['task'] = prompt_text[:clash.TASK_LIMIT]
            opening = clash.opening_prompt(state, prompt_text)
        else:
            state['followup'] = prompt_text[:clash.TASK_LIMIT]
            opening = clash.continue_prompt(state, prompt_text)
        self.emit_state()
        result = self.turn(protagonist_role, opening, mode, 0)
        if not result.ok:
            return self.fail(f'The {protagonist_role} turn did not complete: {result.error}')
        if stage == 'review':
            payload = clash.extract_json(result.text, ('findings',))
            if payload is None:
                return self.fail('Reviewer A did not return structured findings; the clash cannot continue without them.')
            clash.apply_review_turn(state, 'reviewer', payload, 0)
        state['summary'] = clash.prose(result.text, clash.SUMMARY_LIMIT)
        self.emit_state()
        for round_ in range(1, settings['rounds'] + 1):
            state['round'] = round_
            diff = workspace_diff(self.project, self.request.get('baseline')) if stage == 'implement' else None
            result = self.turn('challenger', clash.challenge_prompt(state, round_, diff), 'plan', round_)
            if not result.ok:
                return self.fail(f'The challenger turn did not complete: {result.error}')
            payload = clash.extract_json(result.text, ('verdict', 'objections', 'assessments'))
            if payload is None:
                return self.fail('The challenger did not return a structured verdict; send a follow-up to run another cycle.')
            if stage == 'implement':
                clash.apply_challenge(state, payload, round_)
            else:
                clash.apply_review_turn(state, 'challenger', payload, round_)
            state['turns'][-1]['verdict'] = state['verdict']
            self.emit_state()
            if clash.converged(state) or round_ == settings['rounds']:
                break
            result = self.turn(protagonist_role, clash.response_prompt(state, round_), mode, round_)
            if not result.ok:
                return self.fail(f'The {protagonist_role} response turn did not complete: {result.error}')
            payload = clash.extract_json(result.text, ('responses', 'assessments')) or {}
            if stage == 'implement':
                clash.apply_response(state, payload, round_)
            else:
                clash.apply_review_turn(state, 'reviewer', payload, round_)
            state['summary'] = clash.prose(result.text, clash.SUMMARY_LIMIT)
            self.emit_state()
            if clash.converged(state):
                break
        state['status'] = 'finished'
        state['outcome'] = 'converged' if clash.converged(state) else 'unresolved'
        state['message'] = clash.outcome_message(state)
        state['report'] = clash.render_report(state)
        self.emit_state()
        emit({'kind': 'status', 'text': state['message']})
        emit({'kind': 'result', 'ok': True, 'text': ''})


def main():
    parser = argparse.ArgumentParser()
    parser.parse_args()
    cycle = None
    try:
        raw = sys.stdin.buffer.read(REQUEST_LIMIT + 1)
        if len(raw) > REQUEST_LIMIT:
            raise ValueError('Clash request is too large.')
        request = json.loads(raw)
        if not isinstance(request, dict) or request.get('action') not in ('start', 'continue'):
            raise ValueError('Invalid clash request.')
        cycle = Cycle(request)
        cycle.run()
        return 0
    except Exception as error:
        # Native exceptions may carry private data; report the type and the safe message only.
        message = str(error) if isinstance(error, SessionError) else f'Clash stopped ({type(error).__name__}). Inspect the participants and the project runtime.'
        if cycle is not None:
            cycle.fail(message)
        else:
            emit({'kind': 'error', 'text': message})
            emit({'kind': 'result', 'ok': False, 'text': ''})
        return 1


if __name__ == '__main__':
    sys.exit(main())
