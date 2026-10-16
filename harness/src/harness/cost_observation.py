"""Bounded metadata observations of native launch costs, without retaining content.

Snapshots contain numeric/categorical observations only. Text-component counts
exclude wrappers, nontext contents, unobserved tools and provider tokenization.
Provider-added context, nontext payloads and unobserved tools remain outside scope.
"""
from contextlib import contextmanager
import copy
import hashlib
import hmac
import math
import secrets
import time
from .native_diagnostics import NativeDiagnostics

LIMITS = {"events": 4096, "calls": 512, "characters_per_payload": 200000,
          "content_blocks": 64, "terminals": 8, "identifier_characters": 1024}
COUNTERS = ("input_tokens", "output_tokens", "cached_input_tokens",
            "cache_read_input_tokens", "cache_creation_input_tokens")
COUNTER_LIMIT = 2 ** 63 - 1
EPISODE_LIMIT = 16
CODEX_TOOLS = {"command_execution": "shell", "mcp_tool_call": "mcp",
               "file_change": "edit", "web_search": "search", "collab_tool_call": "helper"}
CURSOR_TOOLS = {"readToolCall": "read", "shellToolCall": "shell", "mcpToolCall": "mcp",
                "editToolCall": "edit", "writeToolCall": "write", "grepToolCall": "search"}


def counter(value):
    return value if type(value) is int and 0 <= value <= COUNTER_LIMIT else None


def money(value):
    if type(value) is int:
        return value if 0 <= value <= COUNTER_LIMIT else None
    return value if type(value) is float and math.isfinite(value) and 0 <= value <= COUNTER_LIMIT else None


class Phase:
    def __init__(self, provider, phase_id, label, *, expected_native=None, limits=None):
        if provider not in ("codex", "claude", "cursor") or label not in ("main", "recovery"):
            raise ValueError("Unsupported observation profile.")
        if type(phase_id) is not int or not 0 <= phase_id <= 4096:
            raise ValueError("Invalid execution identity.")
        self.provider, self.phase_id, self.label = provider, phase_id, label
        self.limits = dict(LIMITS)
        for key, value in (limits or {}).items():
            if key not in LIMITS or type(value) is not int or not 1 <= value <= LIMITS[key]:
                raise ValueError("Invalid observation bound.")
            self.limits[key] = value
        self._key = secrets.token_bytes(32)
        self.expected = self._opaque(expected_native)
        self.native = None
        self.native_conflict = False
        self.native_unknown = expected_native is not None and self.expected is None
        self.events = self.terminal_count = self.occurrences = 0
        self.reports, self.calls, self.seen = [], [], {}
        self.overflow = self.conflict = False
        self.payload_overflow = False
        self.unidentified = self.unverifiable_repeats = self.duplicates = 0

    def _opaque(self, value):
        if not isinstance(value, str) or not value or len(value) > self.limits["identifier_characters"]:
            return None
        return hmac.new(self._key, value.encode("utf-8", "surrogatepass"), hashlib.sha256).digest()

    def _measure(self, content):
        digest = hmac.new(self._key, digestmod=hashlib.sha256)
        complete, overflow, chars, size, inspected = True, False, 0, 0, 0
        if isinstance(content, str):
            digest.update(b"string\0")
            pieces = [content]
        elif isinstance(content, list):
            digest.update(b"blocks\0")
            complete = len(content) <= self.limits["content_blocks"]
            overflow = not complete
            pieces = []
            for block in content[:self.limits["content_blocks"]]:
                if isinstance(block, dict) and block.get("type") == "text" and isinstance(block.get("text"), str):
                    pieces.append(block["text"])
                else:
                    complete = False
        else:
            return {"characters": None, "bytes": None, "known_characters": 0, "known_bytes": 0,
                    "coverage": "unknown"}, None
        for text in pieces:
            remaining = self.limits["characters_per_payload"] - inspected
            if len(text) > remaining:
                complete, overflow = False, True
            text = text[:remaining]
            inspected += len(text)
            digest.update(len(text).to_bytes(8, "big"))
            for start in range(0, len(text), 4096):
                part = text[start:start + 4096]
                try:
                    encoded = part.encode("utf-8")
                except UnicodeError:
                    complete = False
                    continue
                chars += len(part)
                size += len(encoded)
                digest.update(encoded)
        self.payload_overflow |= overflow
        return {"characters": chars if complete else None, "bytes": size if complete else None,
                "known_characters": chars, "known_bytes": size,
                "coverage": "complete" if complete else "partial"}, digest.digest() if complete else None

    def _tool(self, identifier, tool, content, outcome, scope, discriminator=None):
        self.occurrences += 1
        token = self._opaque(identifier)
        entry = self._opaque(discriminator) if discriminator is not None else None
        if discriminator is not None and entry is None:
            token = None
        key = (token, entry) if token is not None else None
        if key not in self.seen and len(self.calls) >= self.limits["calls"]:
            self.overflow = True
            return
        metrics, digest = self._measure(content)
        record = {"tool": tool, "outcome": outcome, "scope": scope, **metrics,
                  "identified": token is not None, "identity_state": "single"}
        if token is None:
            self.unidentified += 1
            record["identity_state"] = "unidentified"
        else:
            if key in self.seen:
                old, signature = self.seen[key]
                new = (tool, scope, outcome, metrics["coverage"], digest)
                if digest is not None and new == signature:
                    self.duplicates += 1
                else:
                    self.conflict = True
                    self.unverifiable_repeats += digest is None
                    old["identity_state"] = "conflict"
                return
            self.seen[key] = (record, (tool, scope, outcome, metrics["coverage"], digest))
        self.calls.append(record)

    def observe(self, event):
        if self.events >= self.limits["events"]:
            self.overflow = True
            return
        self.events += 1
        if not isinstance(event, dict):
            return
        kind = event.get("type")
        field = "thread_id" if self.provider == "codex" else "session_id"
        if field in event:
            identity = self._opaque(event[field])
            if identity is None:
                self.native_unknown = True
            elif self.native not in (None, identity) or self.expected not in (None, identity):
                self.native_conflict = True
            else:
                self.native = identity
        terminal = kind in ("turn.completed", "turn.failed") if self.provider == "codex" else kind == "result"
        if terminal:
            self.terminal_count += 1
            if len(self.reports) >= self.limits["terminals"]:
                self.overflow = True
                return
            raw = event.get("usage") if isinstance(event.get("usage"), dict) else {}
            ok = ((kind == "turn.completed" and not event.get("error") and event.get("is_error") is not True)
                  if self.provider == "codex" else event.get("subtype") == "success" and event.get("is_error") is False)
            self.reports.append({"counters": {key: counter(raw.get(key)) for key in COUNTERS},
                                 "reported_usd": money(event.get("total_cost_usd")),
                                 "outcome": "success" if ok else "failed"})
            return
        if self.provider == "claude" and kind == "user":
            message = event.get("message")
            blocks = message.get("content") if isinstance(message, dict) else None
            if isinstance(blocks, list):
                self.payload_overflow |= len(blocks) > self.limits["content_blocks"]
                scope = ("main_stream" if event["parent_tool_use_id"] is None else "helper_stream") \
                    if "parent_tool_use_id" in event else "unknown"
                for block in blocks[:self.limits["content_blocks"]]:
                    if isinstance(block, dict) and block.get("type") == "tool_result":
                        error = block.get("is_error")
                        outcome = "failed" if error is True else "success" if error is False else "unknown"
                        self._tool(block.get("tool_use_id"), "tool_result", block.get("content"), outcome, scope)
        elif self.provider == "codex" and kind == "item.completed":
            item = event.get("item")
            if isinstance(item, dict) and isinstance(item.get("type"), str) and item["type"] in CODEX_TOOLS:
                name = item["type"]
                body = item.get("aggregated_output") if name == "command_execution" else item.get("result") if name == "mcp_tool_call" else None
                status, exit_code = item.get("status"), item.get("exit_code")
                outcome = ("not_run" if status == "declined" else "failed" if status == "failed" or type(exit_code) is int and exit_code != 0
                           else "success" if status == "completed" else "unknown")
                # Codex structured MCP result schemas are unimplemented, never recursively serialized.
                self._tool(item.get("id"), CODEX_TOOLS[name], body if isinstance(body, str) else None, outcome, "main_stream")
        elif self.provider == "cursor" and kind == "tool_call" and event.get("subtype") == "completed":
            calls = event.get("tool_call")
            if isinstance(calls, dict):
                self.payload_overflow |= len(calls) > self.limits["content_blocks"]
                for number, (name, call) in enumerate(calls.items()):
                    if number >= self.limits["content_blocks"]:
                        break
                    if not isinstance(call, dict):
                        continue
                    result = call.get("result") if isinstance(call.get("result"), dict) else {}
                    success = result.get("success")
                    states = [key for key in ("success", "error", "rejected") if key in result]
                    outcome = {"success": "success", "error": "failed", "rejected": "not_run"}.get(states[0], "unknown") if len(states) == 1 else "unknown"
                    body = success.get("content") if name == "readToolCall" and isinstance(success, dict) else None
                    # This source-defined envelope shares call_id across dictionary entries.
                    self._tool(event.get("call_id"), CURSOR_TOOLS.get(name, "other"), body if isinstance(body, str) else None, outcome, "main_stream", discriminator=name)

    def snapshot(self):
        report = self.reports[0] if self.terminal_count == 1 and len(self.reports) == 1 else None
        counters = report["counters"] if report else {}
        keys = ("input_tokens", "output_tokens") if self.provider == "codex" else \
               ("input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens")
        valid = self.provider != "cursor" and report is not None and all(counters.get(key) is not None for key in keys)
        total = sum(counters[key] for key in keys) if valid else None
        if self.overflow or self.native_conflict or self.native_unknown:
            total = None
        complete_text = not (self.overflow or self.payload_overflow or self.conflict or self.unidentified) \
            and all(row["coverage"] == "complete" for row in self.calls)
        cache_state = "unknown"
        if report and self.provider == "codex" and counters.get("input_tokens") is not None and counters.get("cached_input_tokens") is not None:
            cache_state = "complete" if counters["cached_input_tokens"] <= counters["input_tokens"] else "invalid"
        elif report and self.provider == "claude" and all(counters.get(key) is not None for key in ("cache_read_input_tokens", "cache_creation_input_tokens")):
            cache_state = "complete"
        return {"schema": "private-cost-core-v1", "provider": self.provider, "execution_id": self.phase_id,
                "phase": self.label, "events": self.events, "overflow": self.overflow,
                "payload_overflow": self.payload_overflow, "terminal_count": self.terminal_count,
                "valid_usage_report_count": sum(any(value is not None for value in row["counters"].values()) for row in self.reports),
                "terminal_reports": [{**row, "counters": dict(row["counters"])} for row in self.reports],
                "provider_tokens": total, "token_state": "complete" if total is not None else "incomplete",
                "cache_breakdown_state": cache_state,
                "own_usd": None, "own_usd_state": "unattributed",
                "native_identity_state": "conflict" if self.native_conflict else "unknown" if self.native_unknown else "observed" if self.native else "unobserved",
                "tool_occurrences": self.occurrences, "duplicate_completions": self.duplicates,
                "unidentified_completions": self.unidentified, "unverifiable_repeats": self.unverifiable_repeats,
                "tool_observations": [dict(row) for row in self.calls],
                "tool_component_known_bytes": sum(row["known_bytes"] for row in self.calls if row["identity_state"] == "single"),
                "tool_component_bytes": sum(row["bytes"] for row in self.calls) if complete_text else None,
                "tool_scope": "supported raw text components; excludes wrappers/nontext/unobserved tools; not model tokens"}


def episode(phases):
    if type(phases) not in (list, tuple) or len(phases) > EPISODE_LIMIT:
        return {"executions": [], "known_token_subtotal": None, "provider_tokens": None,
                "token_state": "incomplete", "capacity_exceeded": True}
    if any(type(phase) is not Phase for phase in phases):
        return {"executions": [], "known_token_subtotal": None, "provider_tokens": None,
                "token_state": "incomplete", "capacity_exceeded": False, "invalid_execution": True}
    snapshots = [phase.snapshot() for phase in phases]
    unique = len({row["execution_id"] for row in snapshots}) == len(snapshots)
    known = sum(row["provider_tokens"] for row in snapshots if row["provider_tokens"] is not None) if unique else None
    complete = bool(snapshots) and unique and all(row["provider_tokens"] is not None for row in snapshots)
    return {"executions": snapshots, "known_token_subtotal": known,
            "provider_tokens": known if complete else None, "token_state": "complete" if complete else "incomplete",
            "capacity_exceeded": False}


def text_size(value):
    """Exact constructed text counts when bounded/UTF8; no tokenizer estimate."""
    if not isinstance(value, str):
        return {"characters": None, "bytes": None, "state": "unknown"}
    size = 0
    if len(value) > 2_000_000:
        return {"characters": len(value), "bytes": None, "state": "capacity"}
    for start in range(0, len(value), 4096):
        try:
            size += len(value[start:start + 4096].encode("utf-8"))
        except UnicodeError:
            return {"characters": len(value), "bytes": None, "state": "encoding_unknown"}
    return {"characters": len(value), "bytes": size, "state": "observed"}


def safe(observation, method, *args, **kwargs):
    """Observation failures never change the work or export exception contents."""
    if observation is None:
        return None
    try:
        return getattr(observation, method)(*args, **kwargs)
    except Exception:
        try:
            observation.incomplete = True
            if isinstance(observation, LocalOperation):
                observation.owner.incomplete = True
        except Exception:
            pass
        return None


@contextmanager
def span(observation, name):
    started = time.monotonic()
    try:
        yield
    finally:
        safe(observation, "duration", name, time.monotonic() - started)


class LocalOperation:
    def __init__(self, owner, name):
        self.owner, self.name, self.started = owner, name, time.monotonic()
        self.retries = 0
        self.io_records = []
        self.overflow = False
        self.incomplete = False

    def retry(self):
        self.retries = min(4096, self.retries + 1)

    def io(self, values):
        if len(self.io_records) >= 64:
            self.overflow = True
            return
        counts = ("argv_argument_bytes", "request_file_bytes", "stdout_known_bytes", "stderr_known_bytes",
                  "returned_stdout_bytes", "returned_stderr_bytes")
        flags = ("started", "returned", "stdout_eof", "stderr_eof")
        record = {name: counter(values.get(name)) for name in counts}
        record.update({name: values.get(name) if type(values.get(name)) is bool else None for name in flags})
        record["seconds"] = money(values.get("seconds"))
        self.io_records.append(record)

    def finish(self, result=None, outcome="failed"):
        if isinstance(result, dict):
            outcome = "failed" if result.get("ok") is False else "returned"
        states = {}
        data = result.get("result") if isinstance(result, dict) and isinstance(result.get("result"), dict) else {}
        allowed = {"created", "updated", "existing", "archived", "differs", "archived-differs", "missing", "completed"}
        state_status = "not_applicable"
        if self.name in ("record-result", "replay-result") and outcome == "returned":
            records = data.get("records")
            state_status = "observed"
            if type(records) not in (list, tuple) or len(records) > 128:
                self.incomplete = self.owner.incomplete = True
                state_status = "unknown"
            else:
                for record in records:
                    if (type(record) is dict and type(record.get("state")) is str
                            and record["state"] in allowed):
                        state = record["state"]
                        states[state] = states.get(state, 0) + 1
                    else:
                        self.incomplete = self.owner.incomplete = True
                        state_status = "unknown"
        safe(self.owner, "local", {"operation": self.name, "outcome": outcome,
             "seconds": time.monotonic() - self.started, "lock_retries": self.retries,
             "runtime_io": self.io_records,
             "io_state": "observed_subprocesses" if self.io_records and not self.overflow and not self.incomplete else "unknown",
             "filesystem_io": "unmeasured", "record_states": states, "record_states_state": state_status})


class LaunchObservation:
    """Worker-local generation ownership; only numeric/categorical snapshots persist."""
    def __init__(self, provider, supported=True):
        self.provider, self.supported = provider, bool(supported)
        self.started = time.monotonic()
        self.phases, self.processes, self.payloads, self.usd_reports = {}, {}, {}, {}
        self.diagnostics = {}
        self.durations, self.operations, self.notes = {}, [], {}
        self.remote_operations, self.remote_overflow = [], False
        self.incomplete = False
        self.final = None
        self.outcome = "unknown"
        self.stage = "prepare"
        self.persisted = False

    def new_phase(self, label, native=None):
        if self.supported and self.final is None and label in ("main", "recovery") and label not in self.processes:
            self.processes[label] = {"attempted": True, "started": False, "state": "attempted"}
            try:
                self.diagnostics[label] = NativeDiagnostics(self.provider)
            except Exception:
                self.diagnostics[label] = None
            self.phases[label] = Phase(self.provider, 0 if label == "main" else 1, label, expected_native=native)

    def _diagnostic(self, label, method, *args):
        observed = self.diagnostics.get(label)
        if observed is not None:
            try:
                return getattr(observed, method)(*args)
            except Exception:
                # Diagnostic failure is separate from token-accounting completeness.
                try:
                    observed.unavailable()
                except Exception:
                    self.diagnostics[label] = None
        return None

    def diagnostic(self, label, signal, code=None):
        if self.final is None and label in self.diagnostics:
            self._diagnostic(label, 'boundary', signal, code)

    def process(self, label, state):
        if self.final is None and label in self.processes and state in ("started", "finished", "failed", "cancelled", "skipped"):
            self.processes[label]["started"] |= state == "started"
            self.processes[label]["state"] = state

    def raw(self, label, event):
        if self.final is None:
            self._diagnostic(label, 'observe', event)
            if label in self.phases:
                self.phases[label].observe(event)

    def payload(self, name, value):
        if self.final is None and name in ("main_prompt", "capsule_body", "capsule_envelope", "draft_instruction", "recovery_prompt"):
            self.payloads[name] = text_size(value)

    def transport(self, label, count, state):
        if self.final is None and label in self.processes and state in ("written", "failed", "not_used"):
            self.processes[label]["stdin_written_bytes"] = counter(count)
            self.processes[label]["stdin_state"] = state

    def usd(self, label, own, reported, run_cost):
        if self.final is not None or label not in self.phases:
            return
        kind = "reported_direct"
        if run_cost and run_cost.provider == "claude":
            if run_cost.resumed and run_cost.restores:
                kind = "missing_previous" if run_cost.previous is None else "decreasing_total" if reported < run_cost.previous else "resumed_delta"
            else:
                kind = "legacy_or_unrecognized_version_assumption" if run_cost.resumed else "fresh_reported"
        records = self.usd_reports.setdefault(label, [])
        if len(records) >= 8:
            self.incomplete = True
        else:
            records.append({"own_usd": money(own), "attribution": kind})

    def duration(self, name, seconds):
        if self.final is None and name in ("recall", "prepare", "provider", "recovery", "save", "checkpoint", "history"):
            value = money(seconds)
            if value is not None:
                self.durations[name] = self.durations.get(name, 0) + value

    def operation(self, operation, action=None):
        if operation == "run" and action == "turn":
            action = "checkpoint"
        allowed = {"refresh", "retrieve", "start", "rebind", "record-result", "prepare-result", "replay-result", "brain-create", "brain-update", "promote-auto", "checkpoint", "cursor-rule-retire"}
        name = action if operation == "run" and action in allowed else operation if operation in ("inspect", "info") else "other"
        return LocalOperation(self, name)

    def local(self, record):
        if self.final is not None:
            return
        if len(self.operations) >= 128:
            self.incomplete = True
        else:
            self.operations.append(record)

    def source_hints(self, *, provider_calls, outcome, seconds, references=0, transport=None):
        """Separate optional-remote metadata; unknown billing never alters native totals."""
        if self.final is not None:
            return
        if len(self.remote_operations) >= 16:
            self.remote_overflow = True
            return
        supplied = transport if type(transport) is dict else {}
        def bounded(key, maximum):
            value = supplied.get(key)
            return value if type(value) is int and 0 <= value <= maximum else None
        record = {'schema': 'source-hint-observation-v1', 'operation': 'recall-references',
                  'provider_calls': provider_calls if type(provider_calls) is int and provider_calls in (0, 1) else None,
                  'outcome': outcome if type(outcome) is str and outcome in ('resolved', 'empty', 'invalid', 'unavailable', 'cancelled', 'unsupported') else 'unavailable',
                  'seconds': money(seconds),
                  'supplied_references': references if type(references) is int and 0 <= references <= 8 else None,
                  'transport_method_calls': bounded('attempts', 1),
                  'request_body_bytes': bounded('request_body_bytes', 8192),
                  'response_body_bytes': bounded('response_body_bytes', 65537),
                  'response_eof': supplied.get('response_eof') if type(supplied.get('response_eof')) is bool else None,
                  'remote_tokens': None, 'remote_usd': None, 'provider_tokens': None,
                  'total_cost_complete': False}
        record['metadata_state'] = 'observed_schema_only' if all(record[key] is not None for key in (
            'provider_calls', 'seconds', 'supplied_references', 'transport_method_calls',
            'request_body_bytes', 'response_body_bytes', 'response_eof')) else 'incomplete'
        self.remote_operations.append(record)

    def note(self, name):
        if self.final is None and name in ("stale_revision_retries", "observation_errors"):
            self.notes[name] = min(4096, self.notes.get(name, 0) + 1)

    def set_outcome(self, outcome):
        if self.final is None and outcome in ("completed", "failed", "cancelled", "interrupted", "awaiting_context", "superseded", "not_executed"):
            self.outcome = outcome

    def snapshot(self):
        if self.final is not None:
            return copy.deepcopy(self.final)
        executed = [phase for name, phase in self.phases.items() if self.processes[name]["started"]]
        aggregate = episode(executed)
        if self.incomplete:
            aggregate['provider_tokens'] = None
            aggregate['token_state'] = 'incomplete'
        for row in aggregate["executions"]:
            label = row["phase"]
            attributed = self.usd_reports.get(label) or []
            row["usd_attributions"] = list(attributed)
            if (row["terminal_count"] == 1 and len(attributed) == 1 and attributed[0]["own_usd"] is not None
                    and row["native_identity_state"] not in ("conflict", "unknown") and not row["overflow"]):
                row["own_usd"] = attributed[0]["own_usd"]
                row["own_usd_state"] = "attributed"
        usd_rows = aggregate["executions"]
        aggregate["known_usd_subtotal"] = sum(row["own_usd"] for row in usd_rows if row["own_usd"] is not None)
        usd_complete = bool(usd_rows) and not self.incomplete and all(row["own_usd"] is not None for row in usd_rows)
        aggregate["own_usd"] = aggregate["known_usd_subtotal"] if usd_complete else None
        aggregate["usd_state"] = "complete" if usd_complete else "incomplete"
        return {"schema": "harness-full-cost-v1", "state": "collecting", "outcome": self.outcome,
                "supported_workflow": self.supported, "observer_incomplete": self.incomplete,
                "lifecycle_seconds": time.monotonic() - self.started, "stage_seconds": dict(self.durations),
                "processes": copy.deepcopy(self.processes), "provider": aggregate,
                "native_diagnostics": {label: self._diagnostic(label, 'snapshot') or {
                    'schema': 'native-boundary-diagnostics-v1', 'coverage': 'unavailable',
                    'root_cause': 'unmeasured', 'stderr': 'not_captured'} for label in self.diagnostics},
                "constructed_payloads": copy.deepcopy(self.payloads), "payload_contract": "prompt parts are subsets, not additive; construction/write does not prove model use",
                "local_operations": copy.deepcopy(self.operations), "notes": dict(self.notes),
                "remote_operations": copy.deepcopy(self.remote_operations), "remote_metadata_overflow": self.remote_overflow,
                "remote_cost_contract": "provider totals exclude unknown remote billing; method calls/body bytes do not prove server receipt or model tokens",
                "local_cost_contract": "local I/O/write bytes are not model tokens; filesystem I/O unmeasured"}

    def finish(self, outcome=None):
        if self.final is None:
            for label in self.diagnostics:
                self._diagnostic(label, 'finish')
            if outcome in ("completed", "failed", "cancelled", "interrupted", "awaiting_context", "superseded", "not_executed"):
                self.outcome = outcome
            result = self.snapshot()
            result["state"] = "final"
            self.final = result
        return self.snapshot()


class UnavailableObservation:
    """Fixed fallback when observation initialization failed, without its exception."""
    persisted = False
    incomplete = True

    def finish(self, outcome=None):
        return {"schema": "harness-full-cost-v1", "state": "unavailable", "observer_incomplete": True}
