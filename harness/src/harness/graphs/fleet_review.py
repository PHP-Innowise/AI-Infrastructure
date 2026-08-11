"""Fleet review: parallel multi-lens review of a project, unattended.

The graph mirrors the accelerator's interactive ``/flow-review`` but adds
what only an outer harness can: durable execution (resume a crashed run from
its checkpoint), a human approval gate that can wait hours (``interrupt``),
and a hard cost ceiling enforced between fan-out branches.

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
import operator
import re
from typing import Annotated, Any, TypedDict

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
apply optimizations. Spawn only the {lens} agent from the project roster.

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


class ReviewState(TypedDict, total=False):
    scope: str
    task_id: str
    thread_id: str
    lenses: list[str]
    findings: Annotated[list[dict[str, Any]], operator.add]
    cost_usd: Annotated[float, operator.add]
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
        f"cost: ${state.get('cost_usd', 0.0):.2f}"
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
):
    def scope_node(state: ReviewState) -> dict[str, Any]:
        blackboard.ensure_task(
            state["task_id"], f"Unattended fleet review ({state['scope']})"
        )
        return {"lenses": list(state.get("lenses") or config.lenses)}

    def fan_out(state: ReviewState) -> list[Send]:
        return [
            Send(
                "review",
                {
                    "scope": state["scope"],
                    "task_id": state["task_id"],
                    "thread_id": state["thread_id"],
                    "lens": lens,
                    "cost_so_far": state.get("cost_usd", 0.0),
                },
            )
            for lens in state["lenses"]
        ]

    def review_node(payload: dict[str, Any]) -> dict[str, Any]:
        lens = payload["lens"]
        if payload.get("cost_so_far", 0.0) >= config.budget_usd:
            return {"skipped": [lens]}
        capsule = CAPSULE_TEMPLATE.format(
            lens=lens,
            scope=payload["scope"],
            task_id=payload["task_id"],
            thread_id=payload["thread_id"],
        )
        problems = blackboard.validate_capsule(capsule)
        if problems:
            raise RuntimeError(
                f"delegation capsule for {lens} is under-specified: {problems}"
            )
        blackboard.dispatch_spawn(payload["task_id"], lens, capsule)
        result = worker(WORKER_PROMPT.format(lens=lens, capsule=capsule), config)
        blackboard.dispatch_complete(
            payload["task_id"],
            lens,
            (result.error or " ".join(result.text.split()))[:160] or f"{lens} done",
        )
        if not result.ok:
            return {
                "cost_usd": result.cost_usd,
                "findings": [{
                    "lens": lens,
                    "severity": "high",
                    "claim": f"review worker failed: {result.error}",
                    "file": "",
                    "evidence": "harness",
                }],
            }
        return {
            "cost_usd": result.cost_usd,
            "findings": parse_findings(result.text, lens),
        }

    def collect_node(state: ReviewState) -> dict[str, Any]:
        # Fan-in barrier only: findings accumulate through the reducer, and
        # deduplication happens where the list is consumed (gate/record),
        # because a reducer channel cannot be rewritten in place.
        return {}

    def gate_node(state: ReviewState) -> dict[str, Any]:
        unique = dedupe(state.get("findings", []))
        decision = interrupt({
            "question": "Approve publishing this fleet-review report?",
            "findings": len(unique),
            "high": sum(1 for f in unique if f.get("severity") == "high"),
            "cost_usd": round(state.get("cost_usd", 0.0), 2),
        })
        return {"approved": bool(decision)}

    def record_node(state: ReviewState) -> dict[str, Any]:
        unique = dedupe(state.get("findings", []))
        report = render_report({**state, "findings": unique})
        config.reports_dir.mkdir(parents=True, exist_ok=True)
        path = config.reports_dir / f"{state['thread_id']}.md"
        if state.get("approved"):
            path.write_text(report, encoding="utf-8")
            blackboard.record_progress(
                state["task_id"],
                f"fleet-review {state['thread_id']}: {len(unique)} findings, "
                f"${state.get('cost_usd', 0.0):.2f}, report {path}",
            )
        else:
            blackboard.record_progress(
                state["task_id"],
                f"fleet-review {state['thread_id']}: rejected at the gate "
                f"({len(unique)} findings discarded)",
            )
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
