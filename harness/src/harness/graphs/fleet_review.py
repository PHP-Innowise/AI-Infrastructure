"""Fleet review: parallel multi-lens review of a project, unattended.

The graph mirrors the accelerator's interactive ``/flow-review`` but adds
what only an outer harness can: durable execution (resume a crashed run from
its checkpoint), a human approval gate that can wait hours (``interrupt``),
and per-reviewer allowances delegated to workers with native budget support.

Topology::

    START -> scope -> [Send per lens] review -> collect -> gate -> record -> END

State is plain data (checkpoint-safe); the worker and blackboard are bound
to nodes by closure in ``build_graph``. Each review branch drives one
headless host session whose prompt delegates to the project's OWN roster
agent, so the subagent gate, the Stage B channel, and the SubagentStop
observer all apply inside the worker exactly as they do interactively.
"""

from __future__ import annotations

import json
import math
import operator
import re
from dataclasses import replace
from decimal import Decimal, ROUND_DOWN
from typing import Annotated, Any, Callable, TypedDict

from langgraph.graph import END, START, StateGraph
from langgraph.types import Command, Send, interrupt

from ..blackboard import Blackboard
from ..config import HarnessConfig
from ..workers import Worker

CAPSULE_TEMPLATE = """\
**Objective** - review the change scope below strictly through the \
{lens} lens and report findings.

**Output format** - a JSON array under a `findings` key: each item has \
`file`, `line`, `severity` (high|medium|low), `claim` (one sentence), \
`evidence`. Return the JSON in your final message.

**Tool and source guidance** - work inside the project root; the scope: \
{scope}. Task ID: {task_id}.

**Task boundaries** - report only; do NOT modify files, run fixes, or \
apply optimizations. {delegation}

**Decisions and assumptions so far** - unattended fleet review run \
{thread_id}; no user amendments; severity thresholds are the reviewer's \
judgment.
"""

WORKER_PROMPT = """\
Use the Task tool to spawn the `{lens}` subagent with the delegation \
capsule below as its prompt, wait for it, and then output ONLY the JSON \
findings object from its report (no prose around it).

{capsule}
"""

DIRECT_WORKER_PROMPT = """\
Act as the `{lens}` reviewer yourself. Do not spawn additional agents. \
Follow the delegation capsule below and return ONLY its JSON findings \
object in your final answer.

{capsule}
"""


def sum_costs(left: float | None, right: float | None) -> float | None:
    """One unpriced reviewer makes the total unknown, rather than zero."""
    return None if left is None or right is None else left + right


def cost_label(cost: float | None) -> str:
    return "unknown" if cost is None else f"${cost:.2f}"


class ReviewState(TypedDict, total=False):
    scope: str
    task_id: str
    thread_id: str
    lenses: list[str]
    findings: Annotated[list[dict[str, Any]], operator.add]
    cost_usd: Annotated[float | None, sum_costs]
    skipped: Annotated[list[str], operator.add]
    approved: bool
    report: str


def parse_findings(text: str, lens: str) -> list[dict[str, Any]]:
    """Extract the findings array from a worker's final message, leniently."""
    candidates = [text]
    fenced = re.findall(r"```(?:json)?\s*(.*?)```", text, re.S)
    candidates = fenced + candidates
    for candidate in candidates:
        start = candidate.find("{")
        while start != -1:
            try:
                decoded, _ = json.JSONDecoder().raw_decode(candidate[start:])
            except json.JSONDecodeError:
                start = candidate.find("{", start + 1)
                continue
            if isinstance(decoded, dict) and isinstance(
                decoded.get("findings"), list
            ):
                findings = []
                for item in decoded["findings"]:
                    if isinstance(item, dict) and item.get("claim"):
                        findings.append({**item, "lens": lens})
                return findings
            start = candidate.find("{", start + 1)
    stripped = " ".join(text.split())
    if not stripped:
        return []
    return [{
        "lens": lens,
        "severity": "unparsed",
        "claim": stripped[:300],
        "file": "",
        "evidence": "worker output did not contain a findings object",
    }]


def dedupe(findings: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: set[tuple[str, str]] = set()
    unique: list[dict[str, Any]] = []
    order = {"high": 0, "medium": 1, "low": 2, "unparsed": 3}
    for item in sorted(
        findings, key=lambda f: order.get(str(f.get("severity")), 4)
    ):
        key = (str(item.get("file", "")), str(item.get("claim", ""))[:120].lower())
        if key in seen:
            continue
        seen.add(key)
        unique.append(item)
    return unique


def render_report(state: ReviewState) -> str:
    lines = [
        f"# Fleet review — {state['thread_id']}",
        "",
        f"Scope: {state['scope']}",
        f"Task: {state['task_id']}; lenses: {', '.join(state['lenses'])}; "
        f"cost: {cost_label(state.get('cost_usd', 0.0))}"
        + (
            f"; skipped over budget: {', '.join(state['skipped'])}"
            if state.get("skipped")
            else ""
        ),
        "",
    ]
    findings = state.get("findings", [])
    if not findings:
        lines.append("No findings.")
    for item in findings:
        location = item.get("file") or "-"
        if item.get("line"):
            location += f":{item['line']}"
        lines.append(
            f"- [{item.get('severity', '?')}] {location} ({item.get('lens')}): "
            f"{item.get('claim')}"
            + (f" — {item['evidence']}" if item.get("evidence") else "")
        )
    return "\n".join(lines) + "\n"


def build_graph(
    config: HarnessConfig,
    worker: Worker,
    blackboard: Blackboard,
    checkpointer: Any = None,
    observer: Callable[[dict[str, Any]], None] | None = None,
):
    def stage(name: str, status: str) -> None:
        if observer is not None:
            observer({"kind": "fleet_stage", "stage": name, "status": status})

    def reviewer(lens: str, status: str, cost: float | None = None,
                 duration: float | None = None, error: bool = False, limit_reached: str | None = None) -> None:
        if observer is not None:
            event = {"kind": "fleet_reviewer", "lens": lens, "status": status, "cost_usd": cost}
            if duration is not None:
                event["duration_seconds"] = duration
            if error:
                event["error"] = "Reviewer reached its USD budget; review incomplete." if limit_reached == 'USD' else "Reviewer failed."
            if limit_reached == 'USD':
                event['limit_reached'] = 'USD'
            observer(event)

    def scope_node(state: ReviewState) -> dict[str, Any]:
        stage("scope", "running")
        blackboard.ensure_task(
            state["task_id"], f"Unattended fleet review ({state['scope']})"
        )
        stage("scope", "completed")
        return {"lenses": list(state.get("lenses") or config.lenses)}

    def fan_out(state: ReviewState) -> list[Send]:
        stage("review", "running")
        allowance = None
        if config.budget_usd is not None:
            spent = state.get("cost_usd", 0.0)
            # An unknown previous cost cannot support a new monetary allowance.
            remaining = Decimal(0) if spent is None else max(
                Decimal(0), Decimal(str(config.budget_usd)) - Decimal(str(spent)))
            allowance = float((remaining / len(state["lenses"])).quantize(
                Decimal("0.000001"), rounding=ROUND_DOWN))
        return [
            Send(
                "review",
                {
                    "scope": state["scope"],
                    "task_id": state["task_id"],
                    "thread_id": state["thread_id"],
                    "lens": lens,
                    "budget_usd": allowance,
                },
            )
            for lens in state["lenses"]
        ]

    def review_node(payload: dict[str, Any]) -> dict[str, Any]:
        lens = payload["lens"]
        allowance = payload["budget_usd"]
        if allowance is not None and allowance <= 0:
            reviewer(lens, "skipped", 0.0)
            return {"skipped": [lens], "cost_usd": 0.0}
        reviewer(lens, "running")
        try:
            capsule = CAPSULE_TEMPLATE.format(
                lens=lens,
                scope=payload["scope"],
                task_id=payload["task_id"],
                thread_id=payload["thread_id"],
                delegation=(f"Spawn only the {lens} agent from the project roster."
                            if config.delegate_to_roster else "Do not spawn additional agents."),
            )
            problems = blackboard.validate_capsule(capsule)
            if problems:
                raise RuntimeError(f"delegation capsule for {lens} is under-specified: {problems}")
            blackboard.dispatch_spawn(payload["task_id"], lens, capsule)
            template = WORKER_PROMPT if config.delegate_to_roster else DIRECT_WORKER_PROMPT
            result = worker(template.format(lens=lens, capsule=capsule),
                            replace(config, lenses=(lens,), budget_usd=allowance))
            blackboard.dispatch_complete(
                payload["task_id"], lens,
                (" ".join(result.text.split())[:160] or f"{lens} done")
                if result.ok else f"Reviewer {lens} failed.",
            )
        except Exception:
            reviewer(lens, "failed", error=True)
            raise
        cost = result.cost_usd
        if isinstance(cost, bool) or not isinstance(cost, (int, float)) or not math.isfinite(cost) or cost < 0:
            cost = None
        duration = result.duration_seconds
        if isinstance(duration, bool) or not isinstance(duration, (int, float)) or not math.isfinite(duration) or duration < 0:
            duration = None
        reviewer(lens, "completed" if result.ok else "failed", cost, duration, not result.ok, result.limit_reached)
        if not result.ok:
            # Failed native execution is not a finding about the reviewed code.
            # Checkpoint retry can rerun this node without rerunning completed peers.
            raise RuntimeError(f"Reviewer {lens} reached its USD budget; review incomplete." if result.limit_reached == 'USD' else f"Reviewer {lens} failed.")
        return {
            "cost_usd": cost,
            "findings": parse_findings(result.text, lens),
        }

    def collect_node(state: ReviewState) -> dict[str, Any]:
        # Fan-in barrier only: findings accumulate through the reducer, and
        # deduplication happens where the list is consumed (gate/record),
        # because a reducer channel cannot be rewritten in place.
        stage("review", "completed")
        stage("collect", "running")
        stage("collect", "completed")
        # Emitted by the preceding node once; interrupt() replays the gate on resume.
        stage("gate", "waiting")
        return {}

    def gate_node(state: ReviewState) -> dict[str, Any]:
        unique = dedupe(state.get("findings", []))
        decision = interrupt({
            "question": "Approve publishing this fleet-review report?",
            "findings": len(unique),
            "high": sum(1 for f in unique if f.get("severity") == "high"),
            "cost_usd": (None if state.get("cost_usd") is None
                         else round(state["cost_usd"], 2)),
        })
        stage("gate", "completed")
        return {"approved": bool(decision)}

    def record_node(state: ReviewState) -> dict[str, Any]:
        stage("record", "running")
        unique = dedupe(state.get("findings", []))
        report = render_report({**state, "findings": unique})
        config.reports_dir.mkdir(parents=True, exist_ok=True)
        path = config.reports_dir / f"{state['thread_id']}.md"
        if state.get("approved"):
            path.write_text(report, encoding="utf-8")
            blackboard.record_progress(
                state["task_id"],
                f"fleet-review {state['thread_id']}: {len(unique)} findings, "
                f"cost {cost_label(state.get('cost_usd', 0.0))}, report {path}",
            )
        else:
            blackboard.record_progress(
                state["task_id"],
                f"fleet-review {state['thread_id']}: rejected at the gate "
                f"({len(unique)} findings discarded)",
            )
        stage("record", "completed")
        return {"report": report if state.get("approved") else ""}

    graph = StateGraph(ReviewState)
    graph.add_node("scope", scope_node)
    graph.add_node("review", review_node)
    graph.add_node("collect", collect_node)
    graph.add_node("gate", gate_node)
    graph.add_node("record", record_node)
    graph.add_edge(START, "scope")
    graph.add_conditional_edges("scope", fan_out, ["review"])
    graph.add_edge("review", "collect")
    graph.add_edge("collect", "gate")
    graph.add_edge("gate", "record")
    graph.add_edge("record", END)
    return graph.compile(checkpointer=checkpointer)


def resume_command(approve: bool) -> Command:
    return Command(resume=approve)
