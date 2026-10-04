"""Save to memory: the draft a linked run leaves, and the checks before saving it.

A session linked to a Brain task asks its agent to close each run with a
`memory-draft` block. The agent is the one party that knows what the run
established, and agents left to policy alone almost never record it: on four
real installations 23 of 24 tasks held nothing but the automatic checkpoint.
The draft is only a proposal. The page shows it, a person edits and confirms
it, and only then does the Harness run the runtime's own commands.
"""
from __future__ import annotations

import json
import re
import stat
import uuid

from .filesystem import fs
from .knowledge import _path, _text
from .sessions import SessionError, open_project_path

BLOCK = re.compile(r"```memory-draft[^\S\n]*\n(.*?)\n[^\S\n]*```", re.S)
KINDS = ("finding", "decision")
PROGRESS_LIMIT = 1000
STEP_LIMIT, STEP_COUNT = 300, 3
TITLE_LIMIT, CONSEQUENCE_LIMIT = 200, 1000
LEARNING_COUNT, SOURCE_COUNT = 3, 10
# A reply longer than this is not one a draft can sit at the end of.
TEXT_LIMIT = 200_000

INSTRUCTION = (
    "Project Brain memory draft: this session is linked to a Brain task. End your final "
    "reply with one fenced block whose info string is memory-draft, holding JSON: "
    '{"progress": "where the task stands now, one or two sentences", '
    '"next_steps": ["the next action"], '
    '"learnings": [{"type": "finding" or "decision", "title": "one line", '
    '"consequence": "the reusable rule or decision, one or two sentences", '
    '"sources": ["project-relative file that proves it"]}]}. '
    "Use at most 3 next steps and at most 3 learnings. A learning is durable, reusable "
    "knowledge you verified in a project file during this run, not progress and not a "
    "guess; use [] when there is none. Never include secrets, personal data, logs or "
    "transcripts. A person reviews the draft before anything is saved."
)


def _clip(value, limit):
    return " ".join(value.split())[:limit] if isinstance(value, str) else ""


def parse(text):
    """The last memory-draft block in a reply, clipped to the form's limits, or None.

    Lenient on purpose: this is model output a person is about to review, so a
    learning missing its title is dropped and an overlong field is cut rather
    than the whole draft refused. Saving is where the limits are enforced.
    """
    if not isinstance(text, str) or len(text) > TEXT_LIMIT:
        return None
    blocks = BLOCK.findall(text)
    if not blocks:
        return None
    try:
        data = json.loads(blocks[-1])
    except (ValueError, RecursionError):
        return None
    if not isinstance(data, dict):
        return None
    steps = data.get("next_steps") if isinstance(data.get("next_steps"), list) else []
    learnings = []
    for item in data.get("learnings") if isinstance(data.get("learnings"), list) else []:
        if not isinstance(item, dict) or item.get("type") not in KINDS:
            continue
        sources = item.get("sources") if isinstance(item.get("sources"), list) else []
        learning = {
            "type": item["type"],
            "title": _clip(item.get("title"), TITLE_LIMIT),
            "consequence": _clip(item.get("consequence"), CONSEQUENCE_LIMIT),
            "sources": [source.strip() for source in sources
                        if isinstance(source, str) and source.strip()][:SOURCE_COUNT],
        }
        if learning["title"] and learning["consequence"]:
            learnings.append(learning)
    return {
        "progress": _clip(data.get("progress"), PROGRESS_LIMIT),
        "next_steps": [step for step in (_clip(item, STEP_LIMIT) for item in steps) if step][:STEP_COUNT],
        "learnings": learnings[:LEARNING_COUNT],
    }


def latest(sessions, sid):
    """What the session's last run left for the Save to memory form.

    `drafted` carries the draft; `unreadable` means a block was there but did
    not parse; `missing` means the run replied without one; `none` means no run
    has replied yet. Only the replies after the last user message count, so a
    follow-up never resurrects an earlier run's draft.
    """
    run = []
    for event in sessions.recent_events(sid, ("user", "text", "result"), limit=200):
        if event.get("kind") == "user":
            run = []
        elif isinstance(event.get("text"), str) and event["text"]:
            run.append(event)
    for event in reversed(run):
        draft = parse(event["text"])
        if draft is not None:
            return {"state": "drafted", "draft": draft, "event_id": event.get("id")}
    if any("```memory-draft" in event["text"] for event in run):
        return {"state": "unreadable", "draft": None, "event_id": None}
    return {"state": "missing" if run else "none", "draft": None, "event_id": None}


def _source(value):
    value = _text(value, "learning source", 1024).strip()
    head = value.split("#", 1)[0]
    _path(head)
    return value


def submission(data):
    """A reviewed draft as the page sends it, held to the limits saving relies on."""
    if not isinstance(data, dict) or set(data) - {"progress", "next_steps", "learnings", "verified"}:
        raise SessionError("Invalid memory draft.")
    progress = data.get("progress", "")
    if not isinstance(progress, str) or len(progress) > PROGRESS_LIMIT:
        raise SessionError(f"Write progress in at most {PROGRESS_LIMIT} characters.")
    progress = _text(progress, "progress", PROGRESS_LIMIT * 4).strip() if progress.strip() else ""
    steps = data.get("next_steps", [])
    if not isinstance(steps, list) or len(steps) > STEP_COUNT:
        raise SessionError(f"Keep at most {STEP_COUNT} next steps.")
    steps = [_text(step, "next step", STEP_LIMIT * 4).strip() for step in steps]
    if any(len(step) > STEP_LIMIT for step in steps):
        raise SessionError(f"Write each next step in at most {STEP_LIMIT} characters.")
    learnings = data.get("learnings", [])
    if not isinstance(learnings, list) or len(learnings) > LEARNING_COUNT:
        raise SessionError(f"Keep at most {LEARNING_COUNT} learnings.")
    kept = []
    for item in learnings:
        if (not isinstance(item, dict) or set(item) != {"type", "title", "consequence", "sources"}
                or item["type"] not in KINDS):
            raise SessionError("Each learning needs a type, a title, a consequence and sources.")
        title = _text(item["title"], "learning title", TITLE_LIMIT * 4).strip()
        consequence = _text(item["consequence"], "learning consequence", CONSEQUENCE_LIMIT * 4).strip()
        if len(title) > TITLE_LIMIT or len(consequence) > CONSEQUENCE_LIMIT:
            raise SessionError(f"Keep a learning's title within {TITLE_LIMIT} and its consequence "
                               f"within {CONSEQUENCE_LIMIT} characters.")
        sources = item["sources"]
        if not isinstance(sources, list) or not 1 <= len(sources) <= SOURCE_COUNT:
            raise SessionError(f"Cite 1 to {SOURCE_COUNT} project files for each learning.")
        kept.append({"type": item["type"], "title": title, "consequence": consequence,
                     "sources": list(dict.fromkeys(_source(source) for source in sources))})
    # Saving writes each learning as verified evidence; that word has to be the person's.
    if kept and data.get("verified") is not True:
        raise SessionError("Confirm that you checked each kept learning against its sources.")
    if not progress and not steps and not kept:
        raise SessionError("Nothing to save: write progress, a next step or a learning.")
    return {"progress": progress, "next_steps": steps, "learnings": kept}


def check_sources(root, draft):
    """Refuse a learning citing a file the session workspace does not have.

    The runtime fingerprints every cited source, so a missing one would fail
    halfway through the save; checking first keeps the save whole.
    """
    for learning in draft["learnings"]:
        for source in learning["sources"]:
            head = source.split("#", 1)[0]
            try:
                descriptor = open_project_path(root, head)
            except OSError as error:
                raise SessionError(f"Source not found in the session workspace: {head}") from error
            try:
                if not stat.S_ISREG(fs.fstat(descriptor).st_mode):
                    raise SessionError(f"Source is not a regular file: {head}")
            finally:
                fs.close(descriptor)


def external_id(task_id, kind):
    """A fresh alias that names the task the learning came from."""
    return f"{task_id[:100]}-{kind}-{uuid.uuid4().hex[:8]}"
