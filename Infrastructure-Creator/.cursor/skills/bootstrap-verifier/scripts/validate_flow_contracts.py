#!/usr/bin/env python3
"""Validate schema 1.2 routing and compiled generated flow contracts."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

GRAPH_BLOCK = re.compile(
    r"```json flow-contract\s*\n(?P<payload>.*?)\n```", re.DOTALL
)
AGENT_NAME = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*-agent$")
SKILL_NAME = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
PHASES = {
    "understanding",
    "planning",
    "implementation",
    "verification",
    "finalization",
}


class ContractError(ValueError):
    """Raised when a flow artifact cannot be parsed deterministically."""


def _load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError(f"{path}: cannot read JSON: {error}") from error
    if not isinstance(value, dict):
        raise ContractError(f"{path}: JSON root must be an object")
    return value


def _compiled_block(path: Path) -> dict[str, Any]:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ContractError(f"{path}: cannot read artifact: {error}") from error
    matches = list(GRAPH_BLOCK.finditer(text))
    if len(matches) != 1:
        raise ContractError(
            f"{path}: expected exactly one ```json flow-contract block"
        )
    try:
        value = json.loads(matches[0].group("payload"))
    except json.JSONDecodeError as error:
        raise ContractError(f"{path}: invalid flow-contract JSON: {error}") from error
    if not isinstance(value, dict):
        raise ContractError(f"{path}: flow-contract block must be an object")
    return value


def _frontmatter_stages(path: Path) -> tuple[str, list[dict[str, Any]]]:
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        raise ContractError(f"{path}: missing frontmatter")
    end = text.find("\n---\n", 4)
    if end < 0:
        raise ContractError(f"{path}: unterminated frontmatter")
    lines = text[4:end].splitlines()
    flow = ""
    stages: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for raw in lines:
        stripped = raw.strip()
        if stripped.startswith("flow:"):
            flow = stripped.split(":", 1)[1].strip()
        elif stripped.startswith("- phase:"):
            if current is not None:
                stages.append(current)
            current = {"phase": stripped.split(":", 1)[1].strip()}
        elif current is not None and ":" in stripped:
            key, raw_value = (part.strip() for part in stripped.split(":", 1))
            if key == "agents":
                if not (raw_value.startswith("[") and raw_value.endswith("]")):
                    raise ContractError(f"{path}: agents must use an inline list")
                inner = raw_value[1:-1].strip()
                current[key] = (
                    [item.strip() for item in inner.split(",") if item.strip()]
                    if inner
                    else []
                )
            elif key in {"parallel", "checkpoint"}:
                if raw_value not in {"true", "false"}:
                    raise ContractError(f"{path}: {key} must be true or false")
                current[key] = raw_value == "true"
    if current is not None:
        stages.append(current)
    return flow, stages


def _validate_routing(plan: dict[str, Any], errors: list[str]) -> None:
    skills = plan.get("skills")
    if not isinstance(skills, list):
        errors.append("plan: schema 1.2 requires skills[]")
        return
    names = {
        item.get("name")
        for item in skills
        if isinstance(item, dict) and isinstance(item.get("name"), str)
    }
    expected: dict[str, set[str]] = {}
    for skill in skills:
        if not isinstance(skill, dict) or skill.get("name") not in names:
            continue
        source = skill["name"]
        adjacent: set[str] = set()
        siblings = skill.get("nearest_siblings")
        if not isinstance(siblings, list):
            errors.append(f"routing: {source}.nearest_siblings must be a list")
            continue
        for sibling in siblings:
            name = sibling.get("name") if isinstance(sibling, dict) else None
            if name not in names or name == source:
                errors.append(f"routing: {source} has invalid adjacent skill {name!r}")
            else:
                adjacent.add(name)
        expected[source] = adjacent

    observed: dict[str, set[str]] = {name: set() for name in expected}
    required = {
        "prompt",
        "expected_primary",
        "permitted_secondary",
        "forbidden_skills",
        "rationale",
        "evidence_ids",
    }
    for skill in skills:
        if not isinstance(skill, dict) or skill.get("name") not in names:
            continue
        source = skill["name"]
        cases = skill.get("routing_cases")
        if not isinstance(cases, list) or not cases:
            errors.append(f"routing: {source}.routing_cases must be non-empty")
            continue
        for index, case in enumerate(cases):
            label = f"{source}.routing_cases[{index}]"
            if not isinstance(case, dict) or set(case) != required:
                errors.append(
                    f"routing: {label} must contain exactly {sorted(required)}"
                )
                continue
            primary = case["expected_primary"]
            secondary = case["permitted_secondary"]
            forbidden = case["forbidden_skills"]
            if (
                primary not in ({source} | expected[source])
                or not isinstance(case["prompt"], str)
                or not case["prompt"].strip()
                or not isinstance(secondary, list)
                or any(item not in expected[source] for item in secondary)
                or not isinstance(forbidden, list)
                or any(item not in names for item in forbidden)
                or primary in secondary
                or primary in forbidden
                or set(secondary) & set(forbidden)
            ):
                errors.append(
                    f"routing: {label} is not a deterministic routing oracle"
                )
                continue
            if primary != source:
                observed[source].add(primary)
            observed[source].update(secondary)
    for source in sorted(expected):
        missing = expected[source] - observed.get(source, set())
        extra = observed.get(source, set()) - expected[source]
        if missing:
            errors.append(
                f"routing: {source} omits adjacent skills {sorted(missing)}"
            )
        if extra:
            errors.append(
                f"routing: {source} carries unknown adjacency {sorted(extra)}"
            )


def _validate_graph(
    plan: dict[str, Any], graph: dict[str, Any], errors: list[str]
) -> None:
    if set(graph) != {"roster", "flows"}:
        errors.append("flow_contracts: expected exactly roster and flows")
        return
    skills = plan.get("skills", [])
    expected_roster = [
        {
            "skill": skill.get("name"),
            "agent": f"{skill.get('name')}-agent",
            "phase": skill.get("phase"),
            "writes": bool(skill.get("writes")),
        }
        for skill in skills
        if isinstance(skill, dict) and isinstance(skill.get("name"), str)
    ]
    roster = graph.get("roster")
    flows = graph.get("flows")
    if roster != expected_roster:
        errors.append(
            "flow_contracts: roster must match plan skill order, phases, agents, and writes"
        )
    if not isinstance(flows, list) or not flows:
        errors.append("flow_contracts: flows must be a non-empty list")
        return
    roster_by_agent = {item["agent"]: item for item in expected_roster}
    flow_names: set[str] = set()
    for index, flow in enumerate(flows):
        label = f"flow_contracts.flows[{index}]"
        if not isinstance(flow, dict) or set(flow) != {
            "name",
            "required_code_review",
            "stages",
        }:
            errors.append(
                f"{label}: expected name, required_code_review, and stages"
            )
            continue
        name = flow["name"]
        required_review = flow["required_code_review"]
        stages = flow["stages"]
        if (
            not isinstance(name, str)
            or not SKILL_NAME.fullmatch(name)
            or name in flow_names
        ):
            errors.append(f"{label}: flow name must be valid and unique")
            continue
        flow_names.add(name)
        if not isinstance(stages, list) or not stages:
            errors.append(f"{label}: stages must be a non-empty list")
            continue
        stage_agents: list[str] = []
        checkpoint_count = 0
        previous_phase = -1
        phase_order = [
            "understanding",
            "planning",
            "implementation",
            "verification",
            "finalization",
        ]
        for stage_index, stage in enumerate(stages):
            stage_label = f"{label}.stages[{stage_index}]"
            if not isinstance(stage, dict) or set(stage) != {
                "phase",
                "agents",
                "parallel",
                "checkpoint",
            }:
                errors.append(f"{stage_label}: invalid stage shape")
                continue
            phase = stage["phase"]
            agents = stage["agents"]
            if (
                phase not in PHASES
                or not isinstance(agents, list)
                or not agents
                or any(
                    not isinstance(agent, str)
                    or not AGENT_NAME.fullmatch(agent)
                    or agent not in roster_by_agent
                    for agent in agents
                )
                or not isinstance(stage["parallel"], bool)
                or not isinstance(stage["checkpoint"], bool)
            ):
                errors.append(f"{stage_label}: invalid phase, agents, or flags")
                continue
            order = phase_order.index(phase)
            if order < previous_phase:
                errors.append(f"{stage_label}: phase order moves backwards")
            previous_phase = order
            if len(agents) > 1 and not stage["parallel"]:
                errors.append(f"{stage_label}: multiple agents require parallel true")
            writers = [agent for agent in agents if roster_by_agent[agent]["writes"]]
            if stage["parallel"] and len(writers) > 1:
                errors.append(
                    f"{stage_label}: write-capable agents are not serialized: {writers}"
                )
            checkpoint_count += int(stage["checkpoint"])
            stage_agents.extend(agents)
        if checkpoint_count == 0:
            errors.append(f"{label}: multi-stage flow requires a checkpoint")
        if name == "flow-feature":
            expected_review = (
                "code-review-agent"
                if "code-review-agent" in roster_by_agent
                else None
            )
            if required_review != expected_review:
                errors.append(
                    f"{label}: required_code_review must be {expected_review!r}"
                )
            review_stages = [
                stage_index
                for stage_index, stage in enumerate(stages)
                if isinstance(stage, dict)
                and required_review in stage.get("agents", [])
            ]
            implementation_stages = [
                stage_index
                for stage_index, stage in enumerate(stages)
                if isinstance(stage, dict) and stage.get("phase") == "implementation"
            ]
            if expected_review is None:
                if review_stages:
                    errors.append(
                        f"{label}: unavailable code-review-agent appears in stages"
                    )
            elif not review_stages:
                errors.append(
                    f"{label}: required code-review-agent is absent from stages"
                )
            elif (
                len(review_stages) != 1
                or stages[review_stages[0]].get("phase") != "verification"
                or not implementation_stages
                or review_stages[0] <= max(implementation_stages)
            ):
                errors.append(
                    f"{label}: code-review-agent must occur once in verification "
                    "after implementation"
                )
        elif required_review is not None and required_review not in stage_agents:
            errors.append(f"{label}: required code review is absent from stages")
    missing_flows = {"flow-feature", "flow-review"} - flow_names
    if missing_flows:
        errors.append(
            f"flow_contracts: missing required flows {sorted(missing_flows)}"
        )


def validate_plan_graph(plan: dict[str, Any]) -> list[str]:
    """Validate only the schema 1.2 routing oracle and canonical graph."""
    errors: list[str] = []
    if plan.get("schema_version") != "1.2":
        return ["plan: flow validation requires schema_version 1.2"]
    graph = plan.get("flow_contracts")
    if not isinstance(graph, dict):
        return ["plan: schema 1.2 requires canonical flow_contracts"]
    _validate_routing(plan, errors)
    _validate_graph(plan, graph, errors)
    return sorted(set(errors))


def validate(plan_path: Path, skill_flow: Path, commands_dir: Path) -> list[str]:
    """Return stable, sorted contract errors for a generated flow surface."""
    errors: list[str] = []
    try:
        plan = _load_json(plan_path)
    except ContractError as error:
        return [str(error)]
    errors.extend(validate_plan_graph(plan))
    graph = plan.get("flow_contracts")
    if not isinstance(graph, dict):
        return sorted(set(errors))
    try:
        skill_graph = _compiled_block(skill_flow)
        if skill_graph != graph:
            errors.append("SKILL FLOW: compiled graph differs from flow_contracts")
    except ContractError as error:
        errors.append(str(error))

    flows = graph.get("flows", []) if isinstance(graph.get("flows"), list) else []
    expected_files = {
        f"{flow['name']}.md"
        for flow in flows
        if isinstance(flow, dict) and isinstance(flow.get("name"), str)
    }
    actual_files = (
        {path.name for path in commands_dir.glob("flow-*.md")}
        if commands_dir.is_dir()
        else set()
    )
    if actual_files != expected_files:
        errors.append(
            f"commands: expected {sorted(expected_files)}, found {sorted(actual_files)}"
        )
    roster = graph.get("roster", [])
    for flow in flows:
        if not isinstance(flow, dict) or not isinstance(flow.get("name"), str):
            continue
        path = commands_dir / f"{flow['name']}.md"
        if not path.is_file():
            continue
        expected = {"roster": roster, "flow": flow}
        try:
            compiled = _compiled_block(path)
            if compiled != expected:
                errors.append(
                    f"{path.name}: compiled roster/flow differs from flow_contracts"
                )
            frontmatter_flow, stages = _frontmatter_stages(path)
            if frontmatter_flow != flow["name"]:
                errors.append(f"{path.name}: frontmatter flow name differs")
            if stages != flow["stages"]:
                errors.append(
                    f"{path.name}: frontmatter stage order or content differs"
                )
        except (ContractError, OSError) as error:
            errors.append(str(error))
    return sorted(set(errors))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", required=True)
    parser.add_argument("--skill-flow", required=True)
    parser.add_argument("--commands-dir", required=True)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    errors = validate(
        Path(args.plan), Path(args.skill_flow), Path(args.commands_dir)
    )
    if args.as_json:
        print(json.dumps({"valid": not errors, "errors": errors}, indent=2))
    elif errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
    else:
        print("flow contracts OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
