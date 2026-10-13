"""Composer slash commands, as each native CLI offers them.

Claude Code: `/` at the start of a message. The list is the CLI's own: an `initialize` control request to
`claude -p` in the session's workspace answers with every command and skill it accepts in print mode, without a
model call. A message that starts with one of them goes to the CLI exactly as typed, so Claude Code runs it as when
a person types it. `/clear` (also `/reset` and `/new`), `/model` and `/effort` act on the page instead: a resumed
Harness conversation cannot start over in place, and the page owns the model choice.

Codex: `codex exec` reads every message as plain text, so the Harness does what the Codex TUI would. `$name`
anywhere names one of Codex's own skills (`skills/list` from `codex app-server`, no model call) and gets an
explicit request to load it. `/` at the start offers custom prompts (`/prompts:name`, expanded as the TUI expands
them), `/init`, and `/new`, `/model`, `/diff` and `/status`, which act on the page.

Cursor: `/` at the start lists the project's Cursor skills; a message that starts with one gets an explicit request.

Every CLI: commands its terminal offers and its print mode does not (`/cost`, `/status`, `/resume`, `/memory`,
`/login`, `/help`, `/diff`) open the Harness view that does the same (NAVIGATION). A command of the same name from
the CLI or the project, such as an accelerator's `/memory`, wins.

An accelerator attached to the project from this clone adds its commands and skills as its launch does
(Catalog.catalogue); the composer lists them and a message is routed with them, from the same catalogue.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import selectors
import shlex
import subprocess
import threading
import time

from .filesystem import fs
from . import process_runtime, providers
from .sessions import SessionError, open_project_path, read_context

BASES = {'claude': '.claude/skills', 'codex': '.agents/skills', 'cursor': '.cursor/skills'}
# The folder names the Skills library installs (skills.NAME).
NAME = re.compile(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,99}\Z')
LIMIT = 200
# Codex lists plugin skills too; a person with many plugins has hundreds.
CODEX_LIMIT = 1000
HEAD_BYTES = 16000
DESCRIPTION_CHARS = 300
# How many skills one message can request.
REQUEST_LIMIT = 5
# A probe answers in well under a second; its list is reused for a minute, a failure for ten seconds. A name the
# list lacks is checked against a fresh list once the cached one is a few seconds old (a skill just installed).
PROBE_SECONDS = 10
FRESH_SECONDS = 60
FAILED_SECONDS = 10
RECHECK_SECONDS = 5
PROBE_BYTES = 4 * 1024 * 1024
# Claude Code reads a command name up to the first whitespace.
LEADING = re.compile(r'/(\S+)')
# `$name` after a space or at the start, Codex's spelling of a skill, ending at a space or a sentence mark. Not an
# amount (`US$5`), a path, or code: `($shell)`, `"$origin"`, `$review = 1`, `$item->id` and `$list[0]` stay text.
MENTION = re.compile(r'(?<!\S)\$([a-zA-Z0-9][a-zA-Z0-9._:-]{0,199})(?=$|[\s,;!?)\]}])(?!\s*(?:=|->|::))')
CODE = re.compile(r'```.*?(```|\Z)|`[^`\n]*`', re.S)
# Commands the page carries out itself.
CLAUDE_PAGE = {'clear': 'new', 'reset': 'new', 'new': 'new', 'model': 'model', 'effort': 'effort'}
CODEX_COMMANDS = (
    {'name': 'new', 'description': 'Start a new session in this project', 'hint': '', 'action': 'new'},
    {'name': 'model', 'description': 'Choose the model for the next turn', 'hint': '[model]', 'action': 'model'},
    {'name': 'diff', 'description': "Show this session's changes", 'hint': '', 'action': 'diff'},
    {'name': 'status', 'description': "Show this session's usage", 'hint': '', 'action': 'status'},
    {'name': 'init', 'description': 'Create an AGENTS.md contributor guide for this repository', 'hint': '', 'action': None},
)
# What a CLI's terminal offers and its print mode does not: each opens the Harness view that does the same.
NAVIGATION = (
    {'name': 'status', 'description': "Show this session's usage: tokens, cost and time", 'hint': '', 'action': 'status'},
    {'name': 'cost', 'description': "Show this session's tokens and cost", 'hint': '', 'action': 'status'},
    {'name': 'diff', 'description': "Show this session's changes", 'hint': '', 'action': 'diff'},
    {'name': 'resume', 'description': 'Open an earlier session from the list', 'hint': '', 'action': 'resume'},
    {'name': 'memory', 'description': "Open this project's memory in Knowledge", 'hint': '', 'action': 'memory'},
    {'name': 'login', 'description': 'Say whether the CLI is signed in, and how to sign it in', 'hint': '', 'action': 'login'},
    {'name': 'help', 'description': 'List the commands available here', 'hint': '', 'action': 'help'},
)
INIT_PROMPT = ('Create an AGENTS.md file in the current directory: a short contributor guide for this repository titled '
               '"Repository Guidelines". If AGENTS.md already exists, leave it unchanged and say so. Describe how this '
               'repository actually works: project structure, build, test and development commands, coding style and '
               'naming, testing, and commit and pull request conventions. Keep it to 200-400 words.')
PROMPT_FILES = 200
PROMPT_BYTES = 64 * 1024


def _scalar(value):
    value = re.sub(r'^[|>][-+]?', '', value.strip()).strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in '"\'':
        value = value[1:-1]
    return ' '.join(value.split())


def _short(text, limit=DESCRIPTION_CHARS):
    text = ' '.join(str(text or '').split())
    return text if len(text) <= limit else text[:limit - 1].rstrip() + '…'


def frontmatter(text):
    """Name, description, argument hint and the invocation flags from a leading YAML block; display and routing only."""
    block, body = '', text
    if text.startswith('---'):
        end = text.find('\n---', 3)
        if end >= 0:
            block, body = text[3:end], text[text.find('\n', end + 1) + 1:] if text.find('\n', end + 1) >= 0 else ''
    def field(key):
        # A value runs on over indented lines, as YAML folds them.
        match = re.search(rf'^{key}:[ \t]*(.*?)(?=\n\S|\Z)', block, re.M | re.S)
        return _scalar(match.group(1)) if match else None
    def flag(key, default):
        value = (field(key) or '').lower()
        return value == 'true' if value in ('true', 'false') else default
    return {'name': field('name'), 'description': _short(field('description') or ''), 'hint': _short(field('argument-hint') or '', 200),
            'model': not flag('disable-model-invocation', False), 'user': flag('user-invocable', True), 'body': body}


def workspace_skills(root, provider):
    """The provider's project skills in root, by folder: invocation name, description, SKILL.md path and flags."""
    base = BASES.get(provider)
    if base is None:
        return [], False
    try:
        descriptor = open_project_path(root, base, directory=True)
    except OSError:
        return [], False
    try:
        folders = sorted(fs.listdir(descriptor))
    finally:
        fs.close(descriptor)
    skills, truncated = [], False
    for folder in folders:
        if not NAME.fullmatch(folder):
            continue
        if len(skills) == LIMIT:
            truncated = True
            break
        path = f'{base}/{folder}/SKILL.md'
        content = read_context(root, path, HEAD_BYTES)
        if content is None:
            continue
        meta = frontmatter(content[1])
        name = meta['name'] if meta['name'] and NAME.fullmatch(meta['name']) else folder
        skills.append({'name': name, 'path': path, 'description': meta['description'], 'hint': meta['hint'],
                       'model': meta['model'], 'user': meta['user']})
    return skills, truncated


def converse(command, cwd, env, first, answer, lock_fd=None, timeout=None, owner=None):
    """Run a native CLI briefly over JSON lines: send `first`, pass each JSON object it prints to
    `answer(message, write)` until that returns a result, then close its input and let it exit."""
    timeout = PROBE_SECONDS if timeout is None else timeout
    read_fd, write_fd = os.pipe()
    process, selector, buffer, total, result = None, process_runtime.PipeSelector(), b'', 0, None
    try:
        try:
            process = process_runtime.launch_guarded(command, read_fd, lock_fd=lock_fd, cwd=str(cwd), env=env,
                                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        finally:
            fs.close(read_fd)
        def write(message):
            process.stdin.write((json.dumps(message) + '\n').encode('utf-8'))
            process.stdin.flush()
        for message in first:
            write(message)
        selector.register(process.stdout, selectors.EVENT_READ)
        deadline = time.monotonic() + timeout
        while result is None and selector.get_map():
            if time.monotonic() > deadline:
                raise SessionError('The CLI did not answer in time.')
            for key, _ in selector.select(.1):
                chunk = selector.read(key.fileobj, 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                total += len(chunk)
                if total > PROBE_BYTES:
                    raise SessionError('The CLI answered with more output than expected.')
                buffer += chunk
                while result is None and b'\n' in buffer:
                    line, buffer = buffer.split(b'\n', 1)
                    try:
                        message = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(message, dict):
                        result = answer(message, write)
        if result is None:
            raise SessionError('The CLI exited without answering.')
        return result
    except SessionError:
        raise
    except (OSError, ValueError) as error:
        raise SessionError('The CLI could not be asked for its commands.') from error
    finally:
        if process is not None:
            try:
                process.stdin.close()
            except OSError:
                pass
            # It exits once its input ends, so it can finish writing its own files; closing the watchdog first would
            # have the guard kill it at once. Whatever it left in its process group is stopped afterwards.
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                pass
            process_runtime.reap_tree(process, owner=owner)
            process.stdout.close()
        fs.close(write_fd)
        selector.close()


def claude_settings(agents_enabled=False, agent_count=3, effort=None):
    """The --settings a Claude launch passes (providers.build_command), so the probe sees the same commands."""
    ultracode = effort == 'ultracode'
    settings = {'env': providers.agent_environment('claude', agents_enabled, agent_count), 'ultracode': ultracode}
    if not ultracode:
        settings['disableWorkflows'] = True
    return json.dumps(settings, separators=(',', ':'))


def claude_commands(executable, cwd, settings, lock_fd=None, owner=None, extra=()):
    """Claude Code's own command list for print mode in cwd: name, description, argument hint, aliases, built-in.
    The probe runs no hooks: SessionStart hooks would act on the checkout (and on sessions running in it).
    `extra` carries an attached accelerator's `--add-dir`, whose commands and skills then appear as well."""
    settings = json.dumps({**json.loads(settings), 'disableAllHooks': True}, separators=(',', ':'))
    command = providers.command_argv([executable], 'claude') + [
        '--print', '--input-format', 'stream-json', '--output-format', 'stream-json', '--verbose', '--settings', settings,
        *extra]
    env = {**os.environ}
    env.pop('CONTEXT_CAPSULE_DELIVERED', None)
    def answer(message, write):
        response = message.get('response') if message.get('type') == 'control_response' else None
        if not isinstance(response, dict) or response.get('request_id') != 'harness-commands':
            return None
        if response.get('subtype') != 'success' or not isinstance((response.get('response') or {}).get('commands'), list):
            raise SessionError('Claude Code did not list its commands.')
        return response['response']['commands']
    raw = converse(command, cwd, env, [{'type': 'control_request', 'request_id': 'harness-commands',
                                        'request': {'subtype': 'initialize'}}], answer, lock_fd, owner=owner)
    commands = []
    for item in raw[:CODEX_LIMIT]:
        name = item.get('name') if isinstance(item, dict) else None
        # Underscore names are the CLI's internal handoffs, not commands a person types.
        if not isinstance(name, str) or not name or name.startswith('_') or len(name) > 200 or re.search(r'\s|[\x00-\x1f]', name):
            continue
        aliases = [alias for alias in item.get('aliases') or [] if isinstance(alias, str) and alias and not re.search(r'\s', alias)][:8]
        commands.append({'name': name, 'description': _short(item.get('description')), 'hint': _short(item.get('argumentHint'), 200),
                         'aliases': aliases, 'builtin': item.get('builtin') is True})
    return commands


def codex_skills(executable, cwd, lock_fd=None, owner=None):
    """Codex's own skills for cwd (repository, user, plugin and system), from `codex app-server` `skills/list`."""
    command = providers.command_argv([executable], 'codex') + ['app-server']
    def answer(message, write):
        if message.get('id') == 1:
            if 'error' in message:
                raise SessionError('Codex refused the skills request.')
            write({'jsonrpc': '2.0', 'method': 'initialized'})
            write({'jsonrpc': '2.0', 'id': 2, 'method': 'skills/list', 'params': {'cwds': [str(cwd)]}})
        elif message.get('id') == 2:
            data = (message.get('result') or {}).get('data')
            if 'error' in message or not isinstance(data, list):
                raise SessionError('Codex did not list its skills.')
            return [skill for entry in data if isinstance(entry, dict) for skill in entry.get('skills') or []]
        return None
    raw = converse(command, cwd, {**os.environ}, [{'jsonrpc': '2.0', 'id': 1, 'method': 'initialize',
                                                    'params': {'clientInfo': {'name': 'ai-infrastructure-harness', 'version': '1'}}}],
                   answer, lock_fd, owner=owner)
    skills, seen = [], set()
    for item in raw:
        name, path = (item.get('name'), item.get('path')) if isinstance(item, dict) else (None, None)
        if (not isinstance(name, str) or not isinstance(path, str) or name in seen or item.get('enabled') is False
                or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._:-]{0,199}', name) or len(path) > 4096):
            continue
        seen.add(name)
        skills.append({'name': name, 'description': _short(item.get('description')), 'path': path,
                       'scope': item.get('scope') if isinstance(item.get('scope'), str) else ''})
        if len(skills) == CODEX_LIMIT:
            break
    return skills


def codex_home():
    return Path(os.environ.get('CODEX_HOME') or Path.home() / '.codex')


def codex_prompts(home=None):
    """Codex custom prompts: the top-level Markdown files in $CODEX_HOME/prompts, as `/prompts:<name>`."""
    folder = Path(home or codex_home()) / 'prompts'
    try:
        names = sorted(os.listdir(folder))
    except OSError:
        return []
    prompts = []
    for name in names:
        if not name.endswith('.md') or name.startswith('.') or not NAME.fullmatch(name[:-3]):
            continue
        content = read_context(folder, name, PROMPT_BYTES)
        if content is None or content[0] > PROMPT_BYTES:
            continue
        meta = frontmatter(content[1])
        prompts.append({'name': 'prompts:' + name[:-3], 'description': meta['description'], 'hint': meta['hint'], 'body': meta['body']})
        if len(prompts) == PROMPT_FILES:
            break
    return prompts


def expand_prompt(name, body, arguments):
    """A custom prompt with its arguments, as the Codex TUI expands it: `$1`-`$9` positional, `$ARGUMENTS` all of
    them, `$NAME` from `NAME=value`, `$$` a dollar sign. A named placeholder without its value is an error."""
    try:
        words = shlex.split(arguments)
    except ValueError:
        raise SessionError(f'Close the quotes in the /{name} arguments.') from None
    named = dict(word.split('=', 1) for word in words if re.match(r'[A-Z][A-Z0-9_]*=', word))
    positional = [word for word in words if not re.match(r'[A-Z][A-Z0-9_]*=', word)]
    missing = sorted({key for key in re.findall(r'(?<!\$)\$([A-Z][A-Z0-9_]*)', body.replace('$$', '')) if key != 'ARGUMENTS'} - set(named))
    if missing:
        raise SessionError(f'/{name} needs ' + ', '.join(f'{key}=…' for key in missing) + '.')
    def value(match):
        token = match.group(0)
        if token == '$$':
            return '$'
        key = token[1:]
        if key.isdigit():
            index = int(key) - 1
            return positional[index] if index < len(positional) else ''
        return ' '.join(positional) if key == 'ARGUMENTS' else named.get(key, token)
    return re.sub(r'\$\$|\$[1-9]|\$[A-Z][A-Z0-9_]*', value, body).strip()


def request(skills):
    """The instruction a launch adds for skills the CLI would not load by itself."""
    if not skills:
        return ''
    lines = [f"- {skill['name']} ({skill['path']}): read that file and follow it." for skill in skills]
    return ('\n\nHarness skill request: the user invoked these skills for this message. Load each one before you act '
            'and follow it for this request:\n' + '\n'.join(lines))


class Catalog:
    """Cached command lists per CLI and workspace; one probe at a time for each."""

    def __init__(self, sessions):
        self.sessions = sessions
        self.lock = threading.Lock()
        self.entries = {}
        self.locks = {}

    def clear(self):
        """Forget every list: skills were installed, changed or removed."""
        with self.lock:
            self.entries.clear()

    def _cached(self, key, load, newer_than=None):
        """A list loaded at most FRESH_SECONDS ago, or newer than `newer_than` seconds when that is given."""
        def usable(entry):
            return entry and time.monotonic() < entry[0] and (newer_than is None or time.monotonic() - entry[3] < newer_than)
        with self.lock:
            entry, lock = self.entries.get(key), self.locks.setdefault(key, threading.Lock())
        if usable(entry):
            return entry[1], entry[2]
        with lock:
            with self.lock:
                entry = self.entries.get(key)
            if usable(entry):
                return entry[1], entry[2]
            try:
                value, error, fresh = load(), None, FRESH_SECONDS
            except SessionError as failure:
                value, error, fresh = [], str(failure), FAILED_SECONDS
            with self.lock:
                if len(self.entries) > 64:
                    self.entries.clear()
                    self.locks = {key: lock}
                self.entries[key] = (time.monotonic() + fresh, value, error, time.monotonic())
            return value, error

    def _executable(self, provider):
        found = self.sessions.providers.get(provider) or {}
        if not found.get('available') or not found.get('executable'):
            raise SessionError('This provider CLI is unavailable.')
        return found['executable']

    def claude(self, cwd, settings, newer_than=None, extra=()):
        try:
            executable = self._executable('claude')
        except SessionError as error:
            return [], str(error)
        return self._cached(('claude', executable, os.path.normcase(str(cwd)), settings, tuple(extra)),
                            lambda: claude_commands(executable, cwd, settings, self.sessions.runner_lock, self.sessions, extra),
                            newer_than)

    def codex(self, cwd):
        try:
            executable = self._executable('codex')
        except SessionError as error:
            return [], str(error)
        return self._cached(('codex', executable, os.path.normcase(str(cwd))),
                            lambda: codex_skills(executable, cwd, self.sessions.runner_lock, self.sessions))

    @staticmethod
    def _navigation(commands):
        """The Harness views for terminal-only commands, after the CLI's and the project's own: a name either
        already uses is theirs."""
        taken = {name for item in commands for name in [item['name'], *(item.get('aliases') or [])]}
        return commands + [{**item, 'aliases': [], 'kind': 'page'} for item in NAVIGATION if item['name'] not in taken]

    def catalogue(self, provider, cwd, settings=None, project_id=None, newer_than=None):
        """The entries the CLI in `cwd` takes - Claude Code's commands, or Codex's or Cursor's skills - and an error.
        An accelerator attached to `project_id` adds its own as its launch does: Claude Code is asked with the
        edition's `--add-dir`, and the edition's Codex and Cursor skills follow the CLI's or the project's own, which
        keep their names. The composer lists this and a message is routed against it: what is offered is recognised."""
        home = self.sessions.accelerators.home(project_id, cwd) if project_id else None
        if provider == 'claude':
            return self.claude(cwd, settings or claude_settings(), newer_than, extra=('--add-dir', str(home)) if home else ())
        if provider == 'codex':
            skills, error = self.codex(cwd)
            listed = [{key: item[key] for key in ('name', 'description', 'path')} for item in skills]
        elif provider == 'cursor':
            listed, error = [item for item in workspace_skills(cwd, provider)[0] if item['user']], None
        else:
            raise SessionError('Choose a provider.')
        attached = self.sessions.accelerators.skills(project_id, provider) if home else []
        names = {item['name'] for item in listed}
        return listed + [item for item in attached if item['name'] not in names], error

    def listing(self, provider, cwd, settings=None, project_id=None):
        """What the composer offers: `commands` after a leading `/`, `skills` after `$` (Codex).
        An accelerator attached to `project_id` adds its own, as the launch will (catalogue)."""
        if provider == 'claude':
            native, error = self.catalogue(provider, cwd, settings, project_id)
            commands = [{**item, 'kind': 'page' if item['name'] in CLAUDE_PAGE else 'native',
                         'action': CLAUDE_PAGE.get(item['name'])} for item in native]
            return {'provider': provider, 'commands': self._navigation(commands), 'skills': [], 'error': error}
        if provider == 'codex':
            skills, error = self.catalogue(provider, cwd, project_id=project_id)
            commands = [{'name': item['name'], 'description': item['description'], 'hint': item['hint'], 'aliases': [],
                         'kind': 'page' if item['action'] else 'prompt', 'action': item['action']} for item in CODEX_COMMANDS]
            commands += [{'name': item['name'], 'description': item['description'], 'hint': item['hint'], 'aliases': [],
                          'kind': 'prompt', 'action': None} for item in codex_prompts()]
            return {'provider': provider, 'commands': self._navigation(commands), 'skills': skills, 'error': error}
        if provider == 'cursor':
            skills, error = self.catalogue(provider, cwd, project_id=project_id)
            commands = [{'name': item['name'], 'description': item['description'], 'hint': item.get('hint', ''), 'aliases': [],
                         'kind': 'skill', 'action': None} for item in skills]
            return {'provider': provider, 'commands': self._navigation(commands), 'skills': [], 'error': error}
        raise SessionError('Choose a provider.')

    @staticmethod
    def check(provider, message, page=None):
        """Refuse before a run what cannot work: a command the page carries out (a Claude restart inside a resumed
        conversation, a model or effort the session's own fields own), a Codex prompt that does not exist or lacks
        its arguments. `page` maps the CLI's own aliases to a page action."""
        leading = LEADING.match(message or '')
        if not leading:
            return
        name = leading.group(1)
        action = (page or {}).get(name) or (CLAUDE_PAGE.get(name) if provider == 'claude' else None)
        if provider == 'claude' and action == 'new':
            raise SessionError(f'/{name} would start this conversation over. Use New session instead.')
        if provider == 'claude' and action in ('model', 'effort'):
            raise SessionError(f'The {"model" if action == "model" else "thinking effort"} for the next turn is set in the session settings: '
                               f'send /{name} with only a value, or choose it there.')
        if provider == 'codex' and name.startswith('prompts:'):
            prompt = next((item for item in codex_prompts() if item['name'] == name), None)
            if prompt is None:
                raise SessionError(f'There is no Codex prompt /{name} in {codex_home() / "prompts"}.')
            expand_prompt(name, prompt['body'], message[leading.end():].strip())

    def route(self, session, message):
        """How a message reaches the CLI. `native`: the message goes as typed (a Claude Code command). `text`: the
        message, possibly expanded (a Codex prompt or /init), goes with the Harness context and the skill requests.
        Names resolve against the catalogue the composer lists, an attached accelerator's entries included."""
        provider, cwd, project_id = session['provider'], Path(session['project_path']), session.get('project_id')
        route = {'mode': 'text', 'text': message, 'requests': [], 'notice': None}
        leading = LEADING.match(message or '')
        name = leading.group(1) if leading else None
        if provider == 'claude' and name:
            settings = claude_settings(session['agents_enabled'], session['agent_count'], session['thinking_effort'])
            commands, error = self.catalogue(provider, cwd, settings, project_id)
            known = {alias: item['name'] for item in commands for alias in [item['name'], *item['aliases']]}
            if name not in known and not error:
                commands, error = self.catalogue(provider, cwd, settings, project_id, newer_than=RECHECK_SECONDS)
                known = {alias: item['name'] for item in commands for alias in [item['name'], *item['aliases']]}
            self.check(provider, message, {alias: CLAUDE_PAGE[target] for alias, target in known.items() if target in CLAUDE_PAGE})
            if name in known:
                return {**route, 'mode': 'native', 'notice': f"Claude Code runs /{known[name]} itself: the message went to it as "
                        "typed, without Harness memory or instructions."}
            route['notice'] = (f'/{name} is not a Claude Code command here, so it went as text.' if not error else
                               f'Claude Code could not list its commands ({error}), so /{name} went as text.')
        elif provider == 'codex':
            if name and name.startswith('prompts:'):
                self.check(provider, message)
                prompt = next(item for item in codex_prompts() if item['name'] == name)
                route.update(text=expand_prompt(name, prompt['body'], message[leading.end():].strip()),
                             notice=f'Codex prompt /{name} expanded as the Codex app would.')
            elif name == 'init':
                extra = message[leading.end():].strip()
                route.update(text=INIT_PROMPT + (f'\n\nAlso: {extra}' if extra else ''), notice='Sent Codex /init as its instruction.')
            mentioned = [match.group(1) for match in MENTION.finditer(CODE.sub(' ', message or ''))]
            if mentioned:
                skills, error = self.catalogue(provider, cwd, project_id=project_id)
                by_name = {item['name']: item for item in skills}
                requests = []
                for skill in mentioned:
                    # A sentence may go on right after the name: `$php-review: check rounding`.
                    skill = skill if skill in by_name else skill.rstrip('.:-')
                    if skill in by_name and by_name[skill] not in requests:
                        requests.append(by_name[skill])
                route['requests'] = requests[:REQUEST_LIMIT]
                names = ', '.join(item['name'] for item in route['requests'])
                notice = f'Codex skills requested: {names}.' if names else f'Codex could not list its skills ({error}).' if error else None
                route['notice'] = ' '.join(part for part in (route['notice'], notice) if part) or None
        elif provider == 'cursor' and name:
            skills = {item['name']: item for item in self.catalogue(provider, cwd, project_id=project_id)[0]}
            if name in skills:
                route.update(requests=[skills[name]], notice=f'Cursor skill requested: {name}.')
        return route
