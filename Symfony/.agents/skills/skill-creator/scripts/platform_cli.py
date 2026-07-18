"""Native CLI integration for repository skill evaluation."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
import uuid
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class PlatformConfig:
    key: str
    display_name: str
    project_marker: str
    skills_path: Path
    executable: str
    executable_env: str


def platform_config() -> PlatformConfig:
    parts = Path(__file__).resolve().parts
    if ".agents" in parts:
        return PlatformConfig(
            key="codex",
            display_name="Codex",
            project_marker=".agents",
            skills_path=Path(".agents/skills"),
            executable="codex",
            executable_env="SKILL_CREATOR_CODEX_CLI",
        )
    if ".cursor" in parts:
        return PlatformConfig(
            key="cursor",
            display_name="Cursor",
            project_marker=".cursor",
            skills_path=Path(".cursor/skills"),
            executable="cursor-agent",
            executable_env="SKILL_CREATOR_CURSOR_CLI",
        )
    raise RuntimeError("Cannot infer skill platform from script path.")


def find_project_root(start: Path | None = None) -> Path:
    config = platform_config()
    current = (start or Path.cwd()).resolve()
    for candidate in (current, *current.parents):
        if (candidate / config.project_marker).is_dir():
            return candidate
    raise RuntimeError(
        f"Cannot find a project containing {config.project_marker}/ from {current}."
    )


def _executable(config: PlatformConfig) -> str:
    executable = os.environ.get(config.executable_env, config.executable)
    if os.path.sep in executable:
        if not Path(executable).is_file():
            raise RuntimeError(f"Configured CLI does not exist: {executable}")
    elif shutil.which(executable) is None:
        raise RuntimeError(
            f"{config.display_name} CLI is unavailable. Install/authenticate it or set "
            f"{config.executable_env} to a compatible executable."
        )
    return executable


def _probe_name(skill_name: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", skill_name.lower()).strip("-")[:40]
    return f"{slug or 'skill'}-trigger-probe-{uuid.uuid4().hex[:10]}"


def _write_probe_skill(
    workspace: Path,
    skill_name: str,
    description: str,
) -> tuple[Path, str]:
    config = platform_config()
    probe_name = _probe_name(skill_name)
    marker = f"SKILL_TRIGGERED_{uuid.uuid4().hex.upper()}"
    skill_dir = workspace / config.skills_path / probe_name
    skill_dir.mkdir(parents=True, exist_ok=False)
    skill_md = (
        "---\n"
        f"name: {probe_name}\n"
        f"description: {json.dumps(description)}\n"
        "---\n\n"
        "# Trigger Evaluation Probe\n\n"
        f"When this skill is selected, respond with exactly `{marker}` and stop.\n"
        "Do not call tools and do not complete the user's requested task.\n"
    )
    (skill_dir / "SKILL.md").write_text(skill_md, encoding="utf-8")
    return skill_dir, marker


def evaluate_trigger(
    query: str,
    skill_name: str,
    description: str,
    timeout: int,
    project_root: Path,
    model: str | None,
) -> bool:
    config = platform_config()
    executable = _executable(config)
    del project_root  # Compatibility with the public runner API.

    with tempfile.TemporaryDirectory(prefix="skill-trigger-probe-") as temporary:
        workspace = Path(temporary)
        _, marker = _write_probe_skill(workspace, skill_name, description)
        if config.key == "codex":
            command = [
                executable,
                "exec",
                "--ephemeral",
                "--json",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--cd",
                str(workspace),
            ]
        else:
            command = [executable, "--print", "--output-format", "json"]
        if model:
            command.extend(["--model", model])
        command.append(query)

        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            cwd=workspace,
            env=os.environ.copy(),
            timeout=timeout,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"{config.display_name} trigger probe exited {result.returncode}: "
                f"{result.stderr.strip()}"
            )
        return marker in result.stdout


def call_model(
    prompt: str,
    model: str | None,
    timeout: int = 300,
) -> str:
    config = platform_config()
    executable = _executable(config)
    with tempfile.TemporaryDirectory(prefix="skill-description-improvement-") as temporary:
        workspace = Path(temporary)
        if config.key == "codex":
            output_path = workspace / "last-message.txt"
            command = [
                executable,
                "exec",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--cd",
                str(workspace),
                "--output-last-message",
                str(output_path),
            ]
            if model:
                command.extend(["--model", model])
            command.append("-")
            result = subprocess.run(
                command,
                input=prompt,
                capture_output=True,
                text=True,
                cwd=workspace,
                env=os.environ.copy(),
                timeout=timeout,
            )
            if result.returncode != 0:
                raise RuntimeError(
                    f"Codex description improvement exited {result.returncode}: "
                    f"{result.stderr.strip()}"
                )
            response = (
                output_path.read_text(encoding="utf-8").strip()
                if output_path.is_file()
                else ""
            )
            return response or result.stdout.strip()

        command = [executable, "--print", "--output-format", "json"]
        if model:
            command.extend(["--model", model])
        command.append(prompt)
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            cwd=workspace,
            env=os.environ.copy(),
            timeout=timeout,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"Cursor description improvement exited {result.returncode}: "
                f"{result.stderr.strip()}"
            )
        try:
            payload = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError("Cursor CLI returned invalid JSON output.") from error
        response = payload.get("result")
        if not isinstance(response, str) or not response.strip():
            raise RuntimeError("Cursor CLI JSON output did not contain a non-empty result.")
        return response.strip()
