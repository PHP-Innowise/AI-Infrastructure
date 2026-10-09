"""Save to memory: the draft a linked run leaves, and the checks before saving it.

A session linked to a Brain task asks its agent to close each run with a
`memory-draft` block. The agent is the one party that knows what the run
established, and agents left to policy alone almost never record it: on four
real installations 23 of 24 tasks held nothing but the automatic checkpoint.

By default the Harness saves the draft itself when the run completes, keeping
what the workspace can back and saying in every record that no person reviewed
it. A session opened for review shows the draft instead, and a person edits and
confirms it before the Harness runs the runtime's own commands.
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

REQUEST = (
    "Project Brain memory draft: this session is linked to a Brain task. End your final "
    "reply with one fenced block whose info string is memory-draft, holding JSON: "
    '{"progress": "where the task stands now, one or two sentences", '
    '"next_steps": ["the next action"], '
    '"learnings": [{"type": "finding" or "decision", "title": "one line", '
    '"consequence": "the reusable rule or decision, one or two sentences", '
    '"sources": ["project-relative file that proves it"]}], '
    '"used_memory": ["retrieved capsule path whose claim you checked and used"]}. '
    "Use at most 3 next steps and at most 3 learnings. A learning is durable, reusable "
    "knowledge you verified in a project file during this run, not progress and not a "
    "guess; use [] when there is none. Never include secrets, personal data, logs or "
    "transcripts. "
    "List at most 10 used_memory paths; [] is valid. This is your report of use, not proof of reading. "
)
REVIEWED = "A person reviews the draft before anything is saved."
AUTOMATIC = ("The Harness saves the draft to project memory as written, with no review, "
             "so leave out anything you did not verify.")
# Every record an unattended save writes says so, next to the runtime's own audit trail.
AUTOMATIC_REASON = "Saved automatically when a Harness run completed; agent-attested, not reviewed by a person"


def instruction(review):
    """What a linked run's agent is asked to end with, and who reads it next."""
    return REQUEST + (REVIEWED if review else AUTOMATIC)


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
    result = {
        "progress": _clip(data.get("progress"), PROGRESS_LIMIT),
        "next_steps": [step for step in (_clip(item, STEP_LIMIT) for item in steps) if step][:STEP_COUNT],
        "learnings": learnings[:LEARNING_COUNT],
    }
    if "used_memory" in data:
        values = data["used_memory"] if isinstance(data["used_memory"], list) else []
        result["used_memory"] = list(dict.fromkeys(value.strip() for value in values
            if isinstance(value, str) and value.strip() and len(value) <= 1024))[:SOURCE_COUNT]
    return result


def usage(draft, capsule):
    """Safe provenance: delivered pointers and the subset the agent reports using."""
    delivered = set()
    for layer in ("procedural", "semantic", "episodic", "selected"):
        for item in (capsule or {}).get(layer) or []:
            if isinstance(item, dict) and isinstance(item.get("path"), str):
                delivered.add(item["path"])
    reported = sorted(delivered.intersection((draft or {}).get("used_memory") or []))
    return {"delivered": len(delivered), "reported_used": reported,
            "attestation": "agent-reported", "reported": "used_memory" in (draft or {})}


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
    if not isinstance(data, dict) or set(data) - {"progress", "next_steps", "learnings", "verified", "used_memory"}:
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


def _unusable(root, source):
    """Why the session workspace cannot back a cited source, or None when it can."""
    head = source.split("#", 1)[0]
    try:
        descriptor = open_project_path(root, head)
    except OSError:
        return f"Source not found in the session workspace: {head}"
    try:
        if not stat.S_ISREG(fs.fstat(descriptor).st_mode):
            return f"Source is not a regular file: {head}"
    finally:
        fs.close(descriptor)
    return None


def check_sources(root, draft):
    """Refuse a learning citing a file the session workspace does not have.

    The runtime fingerprints every cited source, so a missing one would fail
    halfway through the save; checking first keeps the save whole.
    """
    for learning in draft["learnings"]:
        for source in learning["sources"]:
            problem = _unusable(root, source)
            if problem:
                raise SessionError(problem)


def remembered(learning):
    """What makes two learnings the same one: their kind and their title, ignoring case."""
    return [learning["type"], " ".join(learning["title"].split()).casefold()]


def usable(root, draft, known=()):
    """What an unattended save keeps of a parsed draft, and the learnings it leaves out.

    Nobody is there to correct a field, so a source the workspace cannot back is
    dropped, a learning left with none is skipped rather than failing the whole
    save, and stray control characters become spaces. A learning already saved
    from this session (`known`, as `remembered` names them) is skipped too: an
    agent restates what it found in later turns. The kept draft still goes
    through `submission`, under the same limits as a reviewed one.
    """
    def plain(value, limit):
        if not isinstance(value, str):
            return ""
        return " ".join("".join(char if 31 < ord(char) != 127 else " " for char in value).split())[:limit]

    known = [list(item) for item in known]
    kept, skipped = [], []
    for item in draft.get("learnings") or []:
        sources = []
        for source in item.get("sources") or []:
            try:
                source = _source(source)
            except SessionError:
                continue
            if _unusable(root, source) is None and source not in sources:
                sources.append(source)
        learning = {"type": item.get("type"), "title": plain(item.get("title"), TITLE_LIMIT),
                    "consequence": plain(item.get("consequence"), CONSEQUENCE_LIMIT), "sources": sources}
        if not (learning["type"] in KINDS and learning["title"] and learning["consequence"] and sources):
            skipped.append({"title": learning["title"] or "an untitled learning", "reason": "unsourced"})
        elif remembered(learning) in known + [remembered(other) for other in kept]:
            skipped.append({"title": learning["title"], "reason": "repeated"})
        else:
            kept.append(learning)
    steps = [step for step in (plain(step, STEP_LIMIT) for step in draft.get("next_steps") or []) if step]
    # The agent attests its own learnings here; the records' ledger says no person did.
    return {"progress": plain(draft.get("progress"), PROGRESS_LIMIT), "next_steps": steps[:STEP_COUNT],
            "learnings": kept[:LEARNING_COUNT], "verified": bool(kept)}, skipped


def summary(state, result=None):
    """One line for the conversation: what an unattended save recorded, or why it did not."""
    if state == "missing":
        return "This run left no memory draft, so nothing was saved to project memory."
    if state == "unreadable":
        return "This run's memory draft could not be read, so nothing was saved to project memory."
    saved = result.get("saved") or {}
    parts = (["the task's progress and next steps"] if saved.get("task") else []) + [
        f"{record.get('type')} \u201c{record.get('title')}\u201d" for record in saved.get("records") or []]
    text = ("Saved to project memory: " + "; ".join(parts) + "." if parts
            else "Nothing new to save to project memory from this run.")
    skipped = [item.get("reason") for item in result.get("skipped") or [] if isinstance(item, dict)]
    if skipped.count("repeated"):
        text += f" {skipped.count('repeated')} learning(s) were already saved from this session."
    if skipped.count("unsourced"):
        text += f" Left out {skipped.count('unsourced')} learning(s) citing no file in the workspace."
    promotion = saved.get("promotion")
    if isinstance(promotion, dict):
        promoted = [item.get("memory_id") for item in promotion.get("promoted") or [] if isinstance(item, dict)]
        held = len(promotion.get("blocked") or []) + len(promotion.get("failed") or [])
        if promotion.get("error"):
            text += " Not promoted yet: " + promotion["error"]
        elif promotion.get("enabled") is False:
            text += " Automatic promotion is off for this project, so it stays in Project Brain."
        if promoted:
            text += " Promoted to the Memory Bank as " + ", ".join(promoted) + "."
        if held:
            text += f" {held} held back from the Memory Bank; see Durable memory."
    if not result.get("ok"):
        text += " Stopped: " + (result.get("error") or "the runtime did not finish.")
        if parts:
            text += " What is listed was saved."
    return text


def external_id(task_id, kind):
    """A fresh alias that names the task the learning came from."""
    return f"{task_id[:100]}-{kind}-{uuid.uuid4().hex[:8]}"
