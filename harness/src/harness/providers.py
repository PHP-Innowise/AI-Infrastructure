"""Stdlib provider boundary for the browser harness; no model calls on import.

Commands follow Claude Code CLI reference, Codex exec/resume --help, and
Cursor CLI parameters/output-format documentation (checked 2026-09-05).
The caller owns cwd, subprocess lifetime, stream limits, and exit-code checks.
"""
from __future__ import annotations

import copy
from contextlib import closing
from datetime import datetime
from functools import lru_cache
import json
import math
import os
from pathlib import Path
import re
import shutil
import sqlite3
import stat
import subprocess
from typing import Optional

PROVIDERS = {"claude": "Claude Code", "codex": "Codex", "cursor": "Cursor Agent"}
PROBE_TIMEOUT = 3
AGENT_CONTROL_DETAILS = {
    "claude": "Requires the selected helper count per turn. Native concurrency limits apply; Ultracode uses its own workflow limits.",
    "codex": "Uses Codex's native limit for concurrent helpers; the main agent runs separately.",
    "cursor": "Instruction only: Cursor has no verified CLI control to disable helpers or cap concurrency.",
}
EFFORTS = {
    "claude": ("low", "medium", "high", "xhigh", "max", "ultracode"),
    "codex": ("none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra", "persistent"),
    "cursor": (),
}
MODEL_CATALOG_LIMIT = 4 * 1024 * 1024


def _value(value: str, label: str) -> str:
    if (not isinstance(value, str) or not value.strip() or value.startswith("-")
            or any(ord(char) < 32 or ord(char) == 127 for char in value)):
        raise ValueError(f"{label} must be a nonempty value, not an option")
    return value


def _probe(executable: str, provider: str) -> bool:
    output = []
    for flag in ("--version", "--help"):
        command = [executable]
        if provider == "cursor":
            # Even help/version can update Cursor's shared `agent` launcher.
            command.append("--disable-auto-update")
        command.append(flag)
        try:
            completed = subprocess.run(
                command, stdin=subprocess.DEVNULL, capture_output=True,
                text=True, errors="replace", timeout=PROBE_TIMEOUT, check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            return False
        if completed.returncode != 0:
            return False
        output.append((completed.stdout + completed.stderr)[:32768])
    identity = {
        "claude": r"\bClaude Code\b", "codex": r"\bcodex(?:-cli| CLI)\b",
        "cursor": r"\bCursor(?: Agent)?\b",
    }[provider]
    if provider == "cursor" and re.search(r"\bgrok\b", output[0], re.IGNORECASE):
        return False
    if not re.search(identity, "\n".join(output), re.IGNORECASE):
        return False
    if provider == "cursor":
        return all(flag in output[1] for flag in (
            "--print", "--output-format", "--resume", "--workspace", "--sandbox", "--mode",
        ))
    return True


def discover_providers(overrides: Optional[dict[str, str]] = None) -> list[dict]:
    """Probe CLI identity with help/version only; never inspect login state."""
    overrides = overrides or {}
    if set(overrides) - set(PROVIDERS):
        raise ValueError("unknown provider override")
    found = []
    for provider, name in PROVIDERS.items():
        if provider in overrides:
            candidates = [_value(overrides[provider], "executable")]
        elif provider == "cursor":
            candidates = ["cursor-agent"]
            for relative in (".local/share/cursor-agent/versions", ".local/share/ai-infrastructure-harness/cursor-agent"):
                candidates.extend(str(path) for path in sorted(
                    (Path.home() / relative).glob("*/cursor-agent"), reverse=True,
                )[:3])
            # `agent` is also used by Grok. It is only eligible after identity checks.
            candidates.append("agent")
        else:
            candidates = [provider]
        executable = None
        seen = set()
        for candidate in candidates:
            path = shutil.which(str(Path(candidate).expanduser()))
            if path is None or path in seen:
                continue
            seen.add(path)
            if _probe(path, provider):
                executable = os.path.abspath(path)
                break
        detail = ("CLI identity verified; login is checked when a run starts."
                  if executable else "CLI not found or identity/required flags could not be verified.")
        if provider == "cursor" and not executable:
            detail += " Set --cursor-bin to a Cursor Agent executable; another tool named agent is not Cursor."
        found.append({"id": provider, "name": name, "available": executable is not None,
                      "executable": executable, "detail": detail,
                      "agent_control_detail": AGENT_CONTROL_DETAILS[provider],
                      "model_options": model_options(provider)})
    return found


def _catalog_rows(data) -> list[dict]:
    """Keep only public model labels and supported effort names from Codex data."""
    if not isinstance(data, dict) or not isinstance(data.get("models"), list):
        return []
    rows = []
    seen = set()
    for entry in data["models"][:1000]:
        if not isinstance(entry, dict) or entry.get("visibility") != "list":
            continue
        identifier, label = entry.get("slug"), entry.get("display_name")
        try:
            _value(identifier, "model")
        except ValueError:
            continue
        if len(identifier) > 120 or identifier in seen:
            continue
        if (not isinstance(label, str) or not label.strip() or len(label) > 120
                or any(ord(char) < 32 or ord(char) == 127 for char in label)):
            label = identifier
        levels = entry.get("supported_reasoning_levels")
        if not isinstance(levels, list):
            continue
        efforts = []
        for level in levels:
            value = level.get("effort") if isinstance(level, dict) else None
            if isinstance(value, str) and re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", value) and value not in efforts:
                efforts.append(value)
        priority = entry.get("priority")
        rows.append((priority if type(priority) is int else 1000,
                     {"id": identifier, "label": label, "efforts": efforts}))
        seen.add(identifier)
    rows.sort(key=lambda row: row[0])
    return [row for _, row in rows[:100]]


def _read_model_cache(path: Path) -> list[dict]:
    descriptor = None
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MODEL_CATALOG_LIMIT:
            return []
        payload = os.read(descriptor, MODEL_CATALOG_LIMIT + 1)
        return _catalog_rows(json.loads(payload)) if len(payload) <= MODEL_CATALOG_LIMIT else []
    except (OSError, ValueError, RecursionError):
        return []
    finally:
        if descriptor is not None:
            os.close(descriptor)


@lru_cache(maxsize=3)
def _model_options(provider: str) -> dict:
    if provider == "claude":
        modern = list(EFFORTS[provider])
        # Explicit IDs and CLI effort levels: code.claude.com/docs/en/model-config
        # Checked 2026-09-05; aliases can resolve differently by provider/settings.
        return {"models": [
            {"id": "claude-fable-5-1", "label": "Fable 5.1", "efforts": modern},
            {"id": "claude-fable-5", "label": "Fable 5", "efforts": modern},
            {"id": "fable", "label": "Fable (auto)", "efforts": modern},
            {"id": "claude-opus-5", "label": "Opus 5", "efforts": modern},
            {"id": "claude-opus-4-8", "label": "Opus 4.8", "efforts": modern},
            {"id": "claude-opus-4-7", "label": "Opus 4.7", "efforts": modern},
            {"id": "claude-opus-4-6", "label": "Opus 4.6", "efforts": ["low", "medium", "high", "max"]},
            {"id": "opus", "label": "Opus (auto)", "efforts": modern},
            {"id": "claude-sonnet-5", "label": "Sonnet 5", "efforts": modern},
            {"id": "claude-sonnet-4-6", "label": "Sonnet 4.6", "efforts": ["low", "medium", "high", "max"]},
            {"id": "sonnet", "label": "Sonnet (auto)", "efforts": modern},
            {"id": "claude-haiku-4-5-20251001", "label": "Haiku 4.5", "efforts": []},
            {"id": "haiku", "label": "Haiku (auto)", "efforts": []},
        ], "efforts": modern,
            "detail": "Versioned entries request an explicit model ID; auto aliases follow your Claude configuration. Account availability and effective effort are checked by Claude; custom model support may differ."}
    if provider == "cursor":
        return {"models": [], "efforts": [],
                "detail": "Use the native default or a custom model/variant. Cursor has no separate effort flag; its account model list requires a native account request."}
    home = Path(os.environ.get("CODEX_HOME") or str(Path.home() / ".codex")).expanduser()
    models = _read_model_cache(home / "models_cache.json")
    source = "Local cached Codex catalog"
    if not models:
        executable = shutil.which("codex")
        if executable and _probe(executable, "codex"):
            try:
                # This documented offline branch returns bundled data before
                # Codex constructs configuration or an authentication manager.
                completed = subprocess.run([executable, "debug", "models", "--bundled"],
                                           stdin=subprocess.DEVNULL, capture_output=True,
                                           timeout=PROBE_TIMEOUT, check=False)
                if completed.returncode == 0 and len(completed.stdout) <= MODEL_CATALOG_LIMIT:
                    models = _catalog_rows(json.loads(completed.stdout))
            except (OSError, ValueError, RecursionError, subprocess.TimeoutExpired):
                pass
        source = "Bundled offline Codex catalog" if models else "No local Codex model catalog is available"
    efforts = list(EFFORTS[provider])
    for model in models:
        efforts.extend(value for value in model["efforts"] if value not in efforts)
    return {"models": models, "efforts": efforts,
            "detail": source + ". Availability is checked by the native CLI; custom models may support different effort levels."}


def model_options(provider: str) -> dict:
    """Return cached public selector metadata; no account or network requests."""
    if provider not in PROVIDERS:
        raise ValueError("unknown provider")
    return copy.deepcopy(_model_options(provider))


def validate_model_effort(provider: str, model: Optional[str], thinking_effort: Optional[str],
                          agents_enabled: bool = False) -> None:
    """Validate known model combinations while retaining custom model IDs."""
    if provider not in PROVIDERS:
        raise ValueError("unknown provider")
    if model is not None:
        _value(model, "model")
    if thinking_effort is None:
        return
    if not isinstance(thinking_effort, str):
        raise ValueError("thinking_effort must be a supported effort name")
    options = model_options(provider)
    levels = options["efforts"]
    for item in options["models"]:
        if item["id"] == model:
            levels = item["efforts"]
            break
    if thinking_effort not in levels:
        raise ValueError("This provider/model does not support the selected thinking effort")
    if thinking_effort == "ultracode" and agents_enabled is not True:
        raise ValueError("Ultracode requires additional agents. Enable them in a new session or choose another effort.")


def input_text(provider: str, prompt: str) -> Optional[str]:
    """Claude/Codex read stdin; Cursor receives one positional argv value."""
    if provider not in PROVIDERS:
        raise ValueError("unknown provider")
    return prompt if provider in ("claude", "codex") else None


def _agent_options(provider: str, enabled: bool, count: int) -> None:
    if provider not in PROVIDERS:
        raise ValueError("unknown provider")
    if type(enabled) is not bool:
        raise ValueError("agents_enabled must be a boolean")
    if type(count) is not int or not 1 <= count <= 40:
        raise ValueError("agent_count must be an integer between 1 and 40")


def agent_environment(provider: str, enabled: bool, count: int) -> dict[str, str]:
    """Return agent-control overrides only; the caller retains its environment."""
    _agent_options(provider, enabled, count)
    if provider == "claude":
        return {
            "CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS": str(count if enabled else 1),
            "CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH": "1",
        }
    return {}


def delegation_instructions(provider: str, enabled: bool, count: int, effort=None) -> str:
    """Shared delegation requirement for ordinary sessions and Creator phases."""
    _agent_options(provider, enabled, count)
    if not enabled:
        return ("Additional agents are disabled for this session. Perform the task in the main agent. "
                "Do not spawn subagents, teammates, workflows, or other agent processes.")
    limit = f"Use at most {count} additional agents concurrently. The main agent does not count toward the required total. "
    if provider == 'claude' and effort == 'ultracode':
        limit += "Ultracode is enabled: use Workflow; native workflow concurrency limits apply. "
    return (f"You MUST launch exactly {count} distinct additional agents during this turn. "
            "This is the user's required total, not an optional maximum. Previous turns' agents, "
            "resuming an existing helper, and messages to the same helper do not count as new launches. "
            "Assign each helper a useful, bounded subtask or an independent check/review. "
            "Do not reduce the count because the task seems easy or fewer helpers would suffice. "
            "Wait for every helper's result and incorporate or assess it before your final answer. "
            + limit + "If native concurrency is lower, launch the remaining helpers in successive batches. "
            "Do not delegate recursively. Keep file writes coordinated and all agents within the shared budgets. "
            "Use only the provider's native delegation tools, never shell-launched agent CLIs as a workaround. "
            "If tools are unavailable, permission is denied, or the remaining budget or task restrictions "
            "prevent the required launches, state the concrete reason and actual/required counts in "
            "your final answer; explicitly mark the delegation requirement unmet. "
            "Never claim that an agent ran without evidence.")


def _codex_helper_activity(event):
    """Codex V2 lifecycle receipts replace V1 spawn_agent collaboration calls."""
    item = event.get('item')
    if (event.get('type') not in ('item.started', 'item.updated', 'item.completed')
            or not isinstance(item, dict) or item.get('type') != 'sub_agent_activity'):
        return None
    phase, identity = item.get('kind'), item.get('agent_thread_id')
    if (phase in ('started', 'completed') and isinstance(identity, str)
            and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.:-]{0,199}', identity)):
        return phase
    return None


def codex_journal_activity(native_id, project, since, until):
    """Read only bounded lifecycle metadata when exec JSON omits V2 activity."""
    if not isinstance(native_id, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.:-]{0,199}', native_id):
        return []
    home = Path(os.environ.get('CODEX_HOME') or str(Path.home() / '.codex')).expanduser()
    try:
        databases = sorted((p for p in home.glob('state_*.sqlite')
                            if re.fullmatch(r'state_[0-9]+\.sqlite', p.name)),
                           key=lambda p: int(p.stem.split('_')[1]), reverse=True)[:3]
        for database in databases:
            if database.is_symlink():
                continue
            try:
                with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=.2)) as db:
                    db.set_progress_handler(lambda: 1, 10000)
                    row = db.execute('SELECT rollout_path,cwd FROM threads WHERE id=?', (native_id,)).fetchone()
            except sqlite3.Error:
                continue
            if row:
                break
        else:
            return []
        path = Path(row[0])
        if str(project) != row[1] or path.suffix != '.jsonl' or path.resolve() != path:
            return []
        if not any(root in path.parents for root in (home / 'sessions', home / 'archived_sessions')):
            return []
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, 'rb') as stream:
            if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                return []
            first = json.loads(stream.readline(65536))
            meta = first.get('payload', {})
            if (first.get('type') != 'session_meta' or not isinstance(meta, dict)
                    or meta.get('id') != native_id or meta.get('cwd') != str(project) or meta.get('source') != 'exec'):
                return []
            # ponytail: inspect only the last 4 MiB; absent/older metadata stays unconfirmed.
            offset = max(stream.tell(), os.fstat(stream.fileno()).st_size - 4 * 1024 * 1024)
            if offset > stream.tell():
                stream.seek(offset); stream.readline(4 * 1024 * 1024)
            tail = stream.read(4 * 1024 * 1024)
        turns, activities = set(), []
        for line in tail.splitlines():
            try:
                record = json.loads(line)
                stamp = datetime.fromisoformat(record.get('timestamp', '').replace('Z', '+00:00'))
                if stamp.tzinfo is None or not since <= stamp.timestamp() <= until or record.get('type') != 'event_msg':
                    continue
                event = record.get('payload', {})
                if not isinstance(event, dict) or not isinstance(event.get('turn_id'), str):
                    continue
                if event.get('type') == 'task_started':
                    turns.add(event['turn_id'])
                item = event.get('item')
                if (event.get('type') == 'item_completed' and event.get('thread_id') == native_id
                        and event['turn_id'] in turns and isinstance(item, dict) and item.get('type') == 'SubAgentActivity'):
                    phase = _codex_helper_activity({'type':'item.completed','item':{**item,'type':'sub_agent_activity'}})
                    activity = {'phase':phase, 'agent_id':item.get('agent_thread_id')}
                    if phase and activity not in activities:
                        activities.append(activity)
            except (ValueError, TypeError, AttributeError, RecursionError):
                continue
        return activities
    except (OSError, ValueError, TypeError, AttributeError, RecursionError):
        return []


class DelegationTracker:
    """Track native tool receipts, not assertions in the model's answer."""
    def __init__(self, provider, required_count=1):
        _agent_options(provider, True, required_count)
        self.provider = provider
        self.required_count = required_count
        self.pending = {}
        self.attempted = False
        self.helpers = set()
        self.journal_confirmed = False

    @property
    def confirmed(self):
        return bool(self.helpers)

    def reconcile(self, native_id, project, since, until):
        if self.provider != 'codex':
            return []
        activities = codex_journal_activity(native_id, project, since, until)
        new_helpers = {item['agent_id'] for item in activities} - self.helpers
        if new_helpers:
            self.attempted = self.journal_confirmed = True
            self.helpers.update(new_helpers)
        return [{'kind':'tool','text':f"Agent activity: {item['phase']} (native session journal)",'ok':True}
                for item in activities if item['agent_id'] in new_helpers]

    def observe(self, event):
        if not isinstance(event, dict):
            return
        if self.provider == 'codex':
            if _codex_helper_activity(event):
                self.attempted = True
                self.helpers.add(event['item']['agent_thread_id'])
            item = event.get('item')
            if (event.get('type') in ('item.started', 'item.updated', 'item.completed')
                    and isinstance(item, dict) and item.get('type') == 'collab_tool_call'
                    and item.get('tool') == 'spawn_agent'):
                self.attempted = True
                receivers = item.get('receiver_thread_ids')
                if (event['type'] == 'item.completed' and item.get('status') == 'completed'
                        and isinstance(receivers, list) and any(isinstance(v, str) and v for v in receivers)):
                    self.helpers.update(v for v in receivers if isinstance(v, str) and v)
            return
        if self.provider != 'claude':
            return  # Cursor does not expose a verified helper-start receipt in our adapter.
        message = event.get('message')
        content = message.get('content') if isinstance(message, dict) else None
        if event.get('type') == 'system' and event.get('subtype') == 'task_started':
            tool_id = event.get('tool_use_id')
            if isinstance(tool_id, str) and self.pending.get(tool_id) in ('Agent', 'Task'):
                self.helpers.add(tool_id)
        if not isinstance(content, list):
            return
        for block in content:
            if not isinstance(block, dict):
                continue
            if event.get('type') == 'assistant' and block.get('type') == 'tool_use':
                name, tool_id = block.get('name'), block.get('id')
                if name in ('Agent', 'Task', 'Workflow') and isinstance(tool_id, str):
                    self.attempted = True
                    inputs = block.get('input')
                    if not isinstance(inputs, dict) or not inputs.get('resume'):
                        self.pending[tool_id] = name
            elif event.get('type') == 'user' and block.get('type') == 'tool_result':
                tool_id = block.get('tool_use_id')
                name = self.pending.pop(tool_id, None) if isinstance(tool_id, str) else None
                if name in ('Agent', 'Task') and block.get('is_error') is not True:
                    self.helpers.add(tool_id)
                # Workflow completion alone may involve zero agents; don't infer a helper launch.

    def summary(self):
        count = len(self.helpers)
        text = f'Confirmed {count}/{self.required_count} required helper launches for this turn. '
        if count == self.required_count:
            status = 'confirmed'
            text += 'The launch count meets the requirement; this does not verify the helpers\' work.'
        elif count > self.required_count:
            status = 'exceeded'
            text += 'More helpers ran than requested; the exact-count requirement was not met.'
        else:
            status = 'partial' if count else 'unconfirmed'
            text += (f'{self.required_count-count} required launches remain unconfirmed. '
                     'The requirement is not confirmed as met; check the model\'s explanation and provider restrictions.')
        if self.journal_confirmed:
            text += ' Includes receipts from the local Codex journal omitted by the CLI stream.'
        if self.attempted and not count:
            text += ' Tool attempts or Workflow calls alone do not confirm individual helpers.'
        if self.provider == 'cursor':
            text += ' Cursor helper telemetry is not verified by this adapter.'
        return {'kind': 'delegation', 'status': status, 'text': text,
                'required_count':self.required_count, 'confirmed_count':count,
                'missing_count':max(0,self.required_count-count)}


def build_command(provider: str, executable: str, project: Path, prompt: str,
                  mode: str = "plan", model: Optional[str] = None,
                  session_id: Optional[str] = None, agents_enabled: bool = False,
                  agent_count: int = 3, thinking_effort: Optional[str] = None, budget_usd: Optional[float] = None) -> list[str]:
    """Construct argv only. The caller must launch with cwd=project and no shell."""
    if provider not in PROVIDERS or mode not in ("plan", "edit"):
        raise ValueError("unknown provider or mode")
    _agent_options(provider, agents_enabled, agent_count)
    validate_model_effort(provider, model, thinking_effort, agents_enabled=agents_enabled)
    _value(executable, "executable")
    project = Path(project).expanduser().resolve()
    if not project.is_dir():
        raise ValueError("project must be an existing directory")
    if not isinstance(prompt, str) or not prompt.strip() or "\0" in prompt:
        raise ValueError("prompt must be nonempty text without NUL characters")
    if model is not None:
        _value(model, "model")
    if session_id is not None:
        _value(session_id, "session ID")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,199}", session_id):
            raise ValueError("invalid native session ID")
    if budget_usd is not None and (provider != 'claude' or type(budget_usd) not in (int,float) or not .01 <= budget_usd <= 1000 or not math.isfinite(budget_usd)):
        raise ValueError('A USD cap of $0.01–$1000 is supported only for Claude.')
    if provider == "claude":
        command = [executable, "--print", "--output-format", "stream-json", "--verbose",
                   "--permission-mode", "plan" if mode == "plan" else "acceptEdits"]
        ultracode = thinking_effort == "ultracode"
        if ultracode:
            # Non-interactive Workflow launches require an explicit allow rule.
            command.extend(["--allowedTools", "Workflow"])
        else:
            command.extend(["--disallowedTools"] + (["Workflow"] if agents_enabled else ["Agent", "Task", "Workflow"]))
        # Settings env outranks inherited env, so apply the cap at CLI scope too.
        environment = agent_environment(provider, agents_enabled, agent_count)
        if thinking_effort is not None:
            # `ultracode` is a CLI mode; the environment accepts real effort only.
            environment["CLAUDE_CODE_EFFORT_LEVEL"] = "xhigh" if ultracode else thinking_effort
            command.extend(["--effort", thinking_effort])
        settings = {"env": environment, "ultracode": ultracode}
        if not ultracode:
            # Also disable inherited/resumed workflow mode when leaving Ultracode.
            settings["disableWorkflows"] = True
        command.extend(["--settings", json.dumps(settings, separators=(",", ":"))])
        if agents_enabled and not ultracode:
            command.extend(["--allowedTools", "Agent", "Task"])
        if session_id:
            command.extend(["--resume", session_id])
    elif provider == "codex":
        # Resume does not accept --sandbox/--cd: global flags precede `exec`.
        command = [executable, "--ask-for-approval", "never", "--sandbox",
                   "read-only" if mode == "plan" else "workspace-write", "--cd", str(project)]
        if agents_enabled:
            # Codex 0.153.2: V1 counts helpers; V2 counts the main thread too.
            # Select V2 explicitly so a model-selected backend cannot change N.
            settings = ["agents.enabled=true", "features.multi_agent=true",
                        f"agents.max_concurrent_threads_per_session={agent_count}",
                        "features.multi_agent_v2={enabled=true,"
                        f"max_concurrent_threads_per_session={agent_count + 1}}}"]
        else:
            settings = ["agents.enabled=false", "features.multi_agent=false",
                        "features.multi_agent_v2=false"]
        for setting in settings:
            command.extend(["-c", setting])
        if thinking_effort is not None:
            command.extend(["-c", "model_reasoning_effort=" + json.dumps(thinking_effort)])
        command.append("exec")
        if session_id:
            command.append("resume")
        command.append("--json")
    else:
        command = [executable, "--disable-auto-update", "--print", "--output-format", "stream-json",
                   "--workspace", str(project), "--sandbox", "enabled"]
        if mode == "plan":
            command.extend(["--mode", "plan"])
        if session_id:
            command.extend(["--resume", session_id])
    if budget_usd is not None:
        command.extend(['--max-budget-usd',str(budget_usd)])
    if model:
        command.extend(["--model", model])
    if provider == "codex":
        if session_id:
            command.append(session_id)
        command.append("-")
    elif provider == "cursor":
        command.extend(["--", prompt])
    return command


def _text(value) -> str:
    return value if isinstance(value, str) else ""


def _usage(event: dict) -> list[dict]:
    usage = event.get("usage")
    result = {"kind": "usage"}
    if isinstance(usage, dict):
        for key in ("input_tokens", "output_tokens", "cached_input_tokens",
                    "cache_read_input_tokens", "cache_creation_input_tokens"):
            value = usage.get(key)
            if type(value) is int and value >= 0:
                result[key] = value
    cost = event.get("total_cost_usd")
    if ((type(cost) is int and cost >= 0)
            or (type(cost) is float and math.isfinite(cost) and cost >= 0)):
        result["cost_usd"] = cost
    return [result] if len(result) > 1 else []


def total_tokens(usage):
    # Codex cached_input_tokens is already inside input_tokens. Claude's cache
    # read/write fields are additional input, per its documented usage contract.
    if any(type(usage.get(k)) is not int or usage[k] < 0 for k in ('input_tokens','output_tokens')):
        return None
    return sum(usage.get(k,0) for k in ('input_tokens','output_tokens','cache_read_input_tokens','cache_creation_input_tokens')
               if type(usage.get(k,0)) is int and usage.get(k,0) >= 0)


def _error(event: dict, fallback: str) -> str:
    error = event.get("error")
    if isinstance(error, dict):
        error = error.get("message")
    errors = event.get("errors")
    if isinstance(errors, list):
        errors = "; ".join(item for item in errors if isinstance(item, str))
    return _text(error) or _text(errors) or _text(event.get("message")) or _text(event.get("result")) or fallback


def normalize_event(provider: str, event: dict) -> list[dict]:
    """Allowlisted public events only; result is terminal, text alone is not.

    A successful result's text is the complete answer, not another text delta.
    Callers should use it as a fallback when no assistant text was received.
    Partial-message flags are intentionally disabled; partial events are ignored.
    """
    if provider not in PROVIDERS:
        raise ValueError("unknown provider")
    if not isinstance(event, dict):
        return []
    kind = event.get("type")
    if not isinstance(kind, str):
        return []
    session_id = event.get("thread_id") if provider == "codex" else event.get("session_id")
    session = {"native_session_id": session_id} if (
        isinstance(session_id, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,199}", session_id)
    ) else {}
    if (provider == "codex" and kind == "thread.started") or (
        provider != "codex" and kind == "system" and event.get("subtype") == "init"
    ):
        return [{"kind": "session", **session}] if session else []
    if provider == "codex" and kind == "turn.started":
        return [{"kind": "status", "text": "Running"}]
    if kind == "error":
        return [{"kind": "error", "text": _error(event, "Provider error"), "ok": False}]
    if (provider == "codex" and kind in ("turn.completed", "turn.failed")) or (
        provider != "codex" and kind == "result"
    ):
        ok = (kind == "turn.completed" and not event.get("error") and event.get("is_error") is not True) or (
            kind == "result" and event.get("subtype") == "success" and event.get("is_error") is False
        )
        text = _text(event.get("result")) if ok else _error(event, "Provider run failed")
        return _usage(event) + [{"kind": "result", "ok": ok, "text": text, **session}]
    if provider == "codex":
        item = event.get("item")
        if not isinstance(item, dict):
            return []
        activity = _codex_helper_activity(event)
        if activity:
            return [{"kind": "tool", "text": f"Agent activity: {activity}", "ok": True}]
        item_type = item.get("type")
        if kind == "item.completed" and item_type == "agent_message":
            text = _text(item.get("text"))
            return [{"kind": "text", "text": text}] if text else []
        if kind in ("item.started", "item.updated", "item.completed") and item_type == "collab_tool_call":
            tool, status = item.get("tool"), item.get("status")
            if tool not in ("spawn_agent", "send_input", "wait", "close_agent") or status not in (
                "in_progress", "completed", "failed",
            ):
                return []
            # Prompts, agent messages and child thread IDs stay inside the CLI.
            return [{"kind": "tool", "text": f"Agent {tool}: {status}", "ok": status != "failed"}]
        if kind in ("item.started", "item.completed") and item_type in (
            "command_execution", "file_change", "mcp_tool_call", "web_search",
        ):
            ok = item.get("status") != "failed" and item.get("exit_code") in (None, 0)
            return [{"kind": "tool", "text": f"{item_type}: {kind.split('.')[1]}", "ok": ok}]
        return []
    if kind == "assistant":
        if event.get("error"):
            return [{"kind": "error", "text": _error(event, "Assistant request failed"), "ok": False}]
        # Cursor partial deltas are not requested; accepting them would duplicate flushes.
        if provider == "cursor" and "timestamp_ms" in event and "model_call_id" not in event:
            return []
        message = event.get("message")
        if not isinstance(message, dict) or message.get("role", "assistant") != "assistant":
            return []
        content = message.get("content")
        if not isinstance(content, list):
            return []
        result = []
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "text" and isinstance(block.get("text"), str):
                result.append({"kind": "text", "text": block["text"]})
            elif block.get("type") == "tool_use":
                result.append({"kind": "tool", "text": (_text(block.get("name")) or "Tool") + ": started"})
        return result
    if kind == "tool_call" and provider == "cursor":
        calls = event.get("tool_call")
        if not isinstance(calls, dict):
            return []
        result = []
        for name, call in calls.items():
            if not isinstance(call, dict):
                continue
            label = _text(call.get("name")) if name == "function" else name
            outcome = call.get("result")
            ok = not (isinstance(outcome, dict) and "error" in outcome)
            phase = "completed" if event.get("subtype") == "completed" else "started"
            result.append({"kind": "tool", "text": f"{label or 'Tool'}: {phase}", "ok": ok})
        return result
    if kind == "user" and provider == "claude":
        message = event.get("message")
        content = message.get("content") if isinstance(message, dict) else None
        if isinstance(content, list):
            return [{"kind": "tool", "text": "Tool: completed", "ok": block.get("is_error") is not True}
                    for block in content if isinstance(block, dict) and block.get("type") == "tool_result"]
    return []
