"""Composer skill hints: the skills a session's provider loads from its workspace, and the skills a message invokes.

Only the provider's own project folder is listed (.claude/skills, .agents/skills, .cursor/skills); personal and
plugin skills are not offered, and skill folders behind a symbolic link are left out. A message invokes a skill
by starting with its name or by naming one the person picked from the list; any other mention is just text. A
launch turns invoked skills into an explicit request (sessions.Sessions._prompt): Claude Code expands a message
that starts with /skill itself, and every other invoked skill gets a line that says how to load it.
"""
from __future__ import annotations

import re

from .filesystem import fs
from .sessions import SessionError, open_project_path, read_context

BASES = {'claude': '.claude/skills', 'codex': '.agents/skills', 'cursor': '.cursor/skills'}
# The folder names the Skills library installs (skills.NAME).
NAME = re.compile(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,99}\Z')
# The names Claude Code accepts for a skill it expands from `/name`.
CLAUDE_NAME = re.compile(r'[a-z0-9]+(?:-[a-z0-9]+)*\Z')
LIMIT = 200
HEAD_BYTES = 16000
DESCRIPTION_CHARS = 300
# How many skills one message can invoke.
REQUEST_LIMIT = 5
# `/name` or `$name` where a word starts, and not followed by more of a path: `src/app`, `US$5`, `/docs/a.md` are text.
REFERENCE = re.compile(r'(?<![^\s(\[{"\'`])([/$])([a-zA-Z0-9][a-zA-Z0-9._-]{0,99})(?![a-zA-Z0-9._/-])')


def _scalar(value):
    value = re.sub(r'^[|>][-+]?', '', value.strip()).strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in '"\'':
        value = value[1:-1]
    return ' '.join(value.split())


def frontmatter(text):
    """Name, description and the two invocation flags from a SKILL.md's leading YAML block; display and routing only."""
    block = ''
    if text.startswith('---'):
        end = text.find('\n---', 3)
        block = text[3:end] if end >= 0 else ''
    def field(key):
        # A value runs on over indented lines, as YAML folds them.
        match = re.search(rf'^{key}:[ \t]*(.*?)(?=\n\S|\Z)', block, re.M | re.S)
        return _scalar(match.group(1)) if match else None
    def flag(key, default):
        value = (field(key) or '').lower()
        return value == 'true' if value in ('true', 'false') else default
    description = field('description') or ''
    if len(description) > DESCRIPTION_CHARS:
        description = description[:DESCRIPTION_CHARS - 1].rstrip() + '…'
    return {'name': field('name'), 'description': description,
            # Claude Code: the Skill tool refuses a skill with disable-model-invocation; user-invocable: false hides it from /.
            'model': not flag('disable-model-invocation', False), 'user': flag('user-invocable', True)}


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
        # Claude Code types the frontmatter name when one is set; the folder otherwise.
        name = meta['name'] if meta['name'] and NAME.fullmatch(meta['name']) else folder
        skills.append({'name': name, 'path': path, 'description': meta['description'],
                       'model': meta['model'], 'user': meta['user']})
    return skills, truncated


def start_only(provider, skill):
    """A Claude skill the Skill tool refuses: only a message that starts with its /name can run it."""
    return provider == 'claude' and not skill['model']


def listing(root, provider):
    """What the composer offers: the skills a person can invoke, as the provider's own / menu shows them."""
    skills, truncated = workspace_skills(root, provider)
    return {'provider': provider, 'base': BASES.get(provider), 'truncated': truncated,
            'skills': [{'name': skill['name'], 'description': skill['description'], 'path': skill['path'],
                        'start_only': start_only(provider, skill)} for skill in skills if skill['user']]}


def validate_picked(value):
    """The names a person picked from the list for one message: at most REQUEST_LIMIT distinct folder-style names."""
    if value is None:
        return []
    if (not isinstance(value, list) or len(value) > REQUEST_LIMIT or len(set(map(str, value))) != len(value)
            or any(not isinstance(name, str) or not NAME.fullmatch(name) for name in value)):
        raise SessionError(f'Pick at most {REQUEST_LIMIT} distinct listed skills.')
    return list(value)


def plan(provider, message, skills, picked=()):
    """How the skills a message invokes reach the agent.

    A skill counts when the message starts with its name, or when the person picked it from the list and the
    message still names it; other mentions ("do not run /deploy yet") stay text. `lead` is the skill Claude
    Code expands by itself: the message starts with exactly `/name` and a space or its end, the way its parser
    reads a command. `request` needs a line in the prompt, and `refused` holds Claude skills the Skill tool
    refuses that do not start the message.
    """
    by_name = {skill['name']: skill for skill in skills if skill['user']}
    message, picked, invoked = message or '', set(picked), []
    for match in REFERENCE.finditer(message):
        # A sentence may end right after the name.
        name = match.group(2) if match.group(2) in by_name else match.group(2).rstrip('.')
        skill = by_name.get(name)
        if not skill or any(item['name'] == name for item in invoked) or not (match.start() == 0 or name in picked):
            continue
        invoked.append({**skill, 'start': match.start(), 'end': match.end(), 'trigger': match.group(1), 'raw': match.group(2)})
        if len(invoked) == REQUEST_LIMIT:
            break
    first = invoked[0] if invoked else None
    lead = first if (provider == 'claude' and first and first['start'] == 0 and first['trigger'] == '/'
                     and first['raw'] == first['name'] and CLAUDE_NAME.fullmatch(first['name']) and len(first['name']) <= 64
                     and (first['end'] == len(message) or message[first['end']].isspace())) else None
    rest = [skill for skill in invoked if skill is not lead]
    return {'lead': lead, 'invoked': invoked, 'request': [skill for skill in rest if not start_only(provider, skill)],
            'refused': [skill for skill in rest if start_only(provider, skill)]}


def request(provider, skills):
    """The instruction a launch adds for invoked skills it cannot leave to the CLI."""
    if not skills:
        return ''
    how = 'invoke it with the Skill tool' if provider == 'claude' else 'read that file and follow it'
    lines = [f"- {skill['name']} ({skill['path']}): {how}." for skill in skills]
    return ('\n\nHarness skill request: the user invoked these skills for this message. Load each one before you act '
            'and follow it for this request:\n' + '\n'.join(lines))


def notice(provider, routed):
    """The conversation's line for the skills a message invoked: which ones, and how each reaches the agent."""
    if not routed['invoked']:
        return None
    text = 'Skills requested in this message: ' + ', '.join(skill['name'] for skill in routed['invoked']) + '.'
    if routed['lead']:
        text += f" Claude Code loads {routed['lead']['name']} from the start of the message."
    if routed['request']:
        names = [skill['name'] for skill in routed['request']]
        text += f" Harness asked the agent to load {'it' if len(names) == 1 else 'them'} before acting: {', '.join(names)}."
    if routed['refused']:
        names = ', '.join(skill['name'] for skill in routed['refused'])
        text += f" Not loaded: {names} runs only from the start of a message (disable-model-invocation)."
    return text
