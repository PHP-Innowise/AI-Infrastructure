"""Portable, curated continuation records for a later AI task.

This module intentionally has no dependency on the SQLite context index or
Project Brain's mutation API.  A handoff is an explicitly authored, portable
artifact, never an attempted export of a client's private conversation.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import subprocess
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath

from validate import SECRET_PATTERNS


class HandoffError(Exception):
    """A safe, user-facing portable-handoff contract violation."""


SCHEMA_VERSION = 1
DETAIL_LIMITS = {"summary": 8_000, "topic": 16_000, "full": 64_000}
SOURCE_CLIENTS = {"codex", "claude", "cursor", "other"}
CURATED_FIELDS = (
    "goal",
    "summary",
    "constraints",
    "decisions",
    "progress",
    "verification",
    "open_questions",
    "next_steps",
    "files",
)
REQUIRED_FIELDS = {"goal", "summary", "next_steps"}
RAW_TRANSCRIPT_PATTERN = re.compile(
    r"^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|"
    r"stdout|stderr|log)\s*:",
    re.IGNORECASE | re.MULTILINE,
)
SENSITIVE_PATH_COMPONENT_PATTERN = re.compile(
    r"^(?:\.git|\.env(?:\..*)?|secrets?|credentials?|id_[a-z0-9]+|"
    r"[^/]+\.(?:pem|key|p12|pfx|jks|keystore))$",
    re.IGNORECASE,
)
INPUT_MAX_BYTES = 1_024 * 1_024
TRANSCRIPT_MAX_BYTES = 4 * 1_024 * 1_024
HANDOFF_MAX_BYTES = 2 * INPUT_MAX_BYTES + TRANSCRIPT_MAX_BYTES
TRANSCRIPT_MARKER = b"\n## Visible transcript\n\n<!-- context-handoff-transcript-v1 -->\n"
INTERNAL_TRANSCRIPT_PATTERN = re.compile(
    r'"(?:type|role)"\s*:\s*"(?:system|developer|tool|reasoning|function_call|tool_call|response_item)"',
    re.IGNORECASE,
)


def _reject_secrets(label: str, values: list[str]) -> None:
    candidate = "\n".join(values)
    for secret_label, pattern in SECRET_PATTERNS.items():
        if pattern.search(candidate):
            raise HandoffError(f"possible {secret_label} detected; {label} not stored")


def _require_string(value: object, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise HandoffError(f"{label} must be a non-empty string")
    return value.strip()


def _require_string_list(value: object, label: str) -> list[str]:
    if not isinstance(value, list) or any(
        not isinstance(item, str) or not item.strip() for item in value
    ):
        raise HandoffError(f"{label} must be a list of non-empty strings")
    return [item.strip() for item in value]


def _reject_symlink_parents(path: Path, *, include_leaf: bool) -> None:
    absolute = path.absolute()
    current = Path(absolute.parts[0])
    parts = absolute.parts[1:] if include_leaf else absolute.parts[1:-1]
    for part in parts:
        current /= part
        if current.is_symlink():
            raise HandoffError(f"handoff path contains a symlink: {path}")


def _read_regular(path: Path, label: str, limit: int) -> bytes:
    _reject_symlink_parents(path, include_leaf=True)
    if not stat.S_ISREG(path.stat().st_mode):
        raise HandoffError(f"{label} must be a regular file")
    if path.stat().st_size > limit:
        raise HandoffError(f"{label} exceeds {limit} bytes")
    with path.open("rb") as handle:
        content = handle.read(limit + 1)
    if len(content) > limit:
        raise HandoffError(f"{label} exceeds {limit} bytes")
    return content


def _read_curated_json(path: Path) -> dict[str, object]:
    try:
        decoded = json.loads(_read_regular(path, "handoff input", INPUT_MAX_BYTES).decode("utf-8"))
    except (OSError, UnicodeDecodeError, ValueError, RecursionError) as error:
        raise HandoffError("handoff input must contain one JSON object") from error
    if not isinstance(decoded, dict):
        raise HandoffError("--input must be one JSON object")
    return decoded


def validate_curated_context(decoded: dict[str, object], detail: str) -> dict[str, object]:
    """Decode and validate the deliberately small curated continuation schema."""
    if detail not in DETAIL_LIMITS:
        raise HandoffError(f"detail must be one of: {', '.join(DETAIL_LIMITS)}")
    unexpected = set(decoded) - set(CURATED_FIELDS)
    missing = REQUIRED_FIELDS - set(decoded)
    if unexpected:
        raise HandoffError(f"input has unexpected fields: {', '.join(sorted(unexpected))}")
    if missing:
        raise HandoffError(f"input is missing required fields: {', '.join(sorted(missing))}")

    context: dict[str, object] = {}
    for field in CURATED_FIELDS:
        raw = decoded.get(field, [] if field not in {"goal", "summary"} else None)
        if field in {"goal", "summary"}:
            context[field] = _require_string(raw, field)
        elif field == "files":
            if not isinstance(raw, list) or any(
                not isinstance(item, str) or not item or not item.strip() for item in raw
            ):
                raise HandoffError("files must be a list of non-empty strings")
            context[field] = list(raw)
        else:
            context[field] = _require_string_list(raw, field)
    if not context["next_steps"]:
        raise HandoffError("next_steps must not be empty")

    text_values = [context["goal"], context["summary"]]
    for field in CURATED_FIELDS[2:]:
        text_values.extend(context[field])  # type: ignore[arg-type]
    authored = "\n".join(text_values)
    if len(authored) > DETAIL_LIMITS[detail]:
        raise HandoffError(
            f"curated context exceeds the {detail} limit of {DETAIL_LIMITS[detail]} characters"
        )
    _reject_secrets("curated context", text_values)
    if RAW_TRANSCRIPT_PATTERN.search(authored):
        raise HandoffError("curated context contains raw transcript-like data; replace it with a summary")
    return context


def _git_value(repository: Path, arguments: list[str]) -> str | None:
    try:
        completed = subprocess.run(
            ["git", "-C", str(repository), *arguments],
            capture_output=True,
            text=True,
            check=False,
            timeout=2,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return completed.stdout.strip() if completed.returncode == 0 else None


def _safe_relative_path(repository: Path, value: str) -> tuple[str, Path]:
    if not value or value != value.strip() or "\\" in value or any(ord(char) < 32 for char in value):
        raise HandoffError("file path must be a non-empty, trimmed repository-relative path")
    pure = PurePosixPath(value)
    if pure.is_absolute() or ".." in pure.parts or value.startswith("./") or ":" in pure.parts[0]:
        raise HandoffError(f"file path escapes the repository: {value}")
    normalized = pure.as_posix()
    if normalized in {"", "."} or any(
        SENSITIVE_PATH_COMPONENT_PATTERN.fullmatch(part) for part in pure.parts
    ):
        raise HandoffError(f"file path is sensitive or invalid: {value}")
    path = repository.joinpath(*pure.parts)
    current = repository
    for part in pure.parts:
        current = current / part
        if current.is_symlink():
            raise HandoffError(f"file path contains a symlink: {value}")
    return normalized, path


def _is_ignored(repository: Path, relative: str) -> bool:
    try:
        completed = subprocess.run(
            ["git", "-C", str(repository), "check-ignore", "--quiet", "--", relative],
            capture_output=True,
            text=True,
            check=False,
            timeout=2,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise HandoffError("cannot verify source ignore rules") from error
    if completed.returncode not in (0, 1):
        raise HandoffError("cannot verify source ignore rules")
    return completed.returncode == 0


def fingerprint_files(repository: Path, values: list[str]) -> list[dict[str, object]]:
    if len(values) > 256:
        raise HandoffError("at most 256 source files may be referenced")
    fingerprints: list[dict[str, object]] = []
    seen: set[str] = set()
    for value in values:
        relative, path = _safe_relative_path(repository, value)
        if relative in seen:
            raise HandoffError(f"file paths must be unique: {relative}")
        seen.add(relative)
        if _is_ignored(repository, relative):
            raise HandoffError(f"file path is ignored and cannot be included: {relative}")
        if path.exists() and not path.is_file():
            raise HandoffError(f"file path is not a regular file: {relative}")
        digest = None
        if path.is_file():
            checksum = hashlib.sha256()
            with path.open("rb") as handle:
                for block in iter(lambda: handle.read(65536), b""):
                    checksum.update(block)
            digest = checksum.hexdigest()
        fingerprints.append({"path": relative, "sha256": digest})
    return fingerprints


def _render_list(items: list[str]) -> str:
    return "\n".join(f"- {item}" for item in items) if items else "- None"


def render_body(context: dict[str, object]) -> str:
    return (
        "# Context handoff\n\n"
        f"## Goal\n\n{context['goal']}\n\n"
        f"## Summary\n\n{context['summary']}\n\n"
        f"## Constraints\n\n{_render_list(context['constraints'])}\n\n"
        f"## Decisions\n\n{_render_list(context['decisions'])}\n\n"
        f"## Progress\n\n{_render_list(context['progress'])}\n\n"
        f"## Verification\n\n{_render_list(context['verification'])}\n\n"
        f"## Open questions\n\n{_render_list(context['open_questions'])}\n\n"
        f"## Next steps\n\n{_render_list(context['next_steps'])}\n\n"
        f"## Files\n\n{_render_list(context['files'])}\n"
    )


def _read_transcript(path: Path) -> bytes:
    if path.suffix.lower() not in {".md", ".markdown", ".txt"}:
        raise HandoffError("transcript must be a visible .txt or Markdown export")
    try:
        transcript = _read_regular(path, "transcript", TRANSCRIPT_MAX_BYTES)
        text = transcript.decode("utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise HandoffError("transcript must be valid UTF-8 text") from error
    _reject_secrets("transcript", [text])
    if not text.strip():
        raise HandoffError("transcript must not be empty")
    # Reject an internal JSON/JSONL export, not a visible discussion containing
    # a fenced JSON example. Such examples are legitimate conversation text.
    if all(line.lstrip().startswith("{") for line in text.splitlines() if line.strip()) and INTERNAL_TRANSCRIPT_PATTERN.search(text):
        raise HandoffError("transcript appears to be internal reasoning or tool JSONL, not a visible client export")
    return transcript


def _load_markdown(path: Path) -> tuple[dict[str, object], bytes]:
    try:
        text = _read_regular(path, "handoff", HANDOFF_MAX_BYTES)
        decoded = text.decode("utf-8")
    except OSError as error:
        raise HandoffError(f"cannot read handoff: {path}") from error
    except UnicodeDecodeError as error:
        raise HandoffError("handoff must be valid UTF-8") from error
    _reject_secrets("handoff", [decoded])
    if not text.startswith(b"---\n"):
        raise HandoffError("handoff is missing JSON frontmatter")
    try:
        raw_metadata, body = text[4:].split(b"\n---\n", 1)
        metadata = json.loads(raw_metadata.decode("utf-8"))
    except (ValueError, UnicodeDecodeError, RecursionError) as error:
        raise HandoffError("handoff has invalid JSON frontmatter") from error
    if not isinstance(metadata, dict):
        raise HandoffError("handoff frontmatter must be an object")
    return metadata, body


def _validate_metadata(metadata: dict[str, object], body: bytes) -> tuple[dict[str, object], bytes | None]:
    required = {
        "schema_version", "type", "created_at", "detail", "topic", "source_client",
        "task_id", "repository", "files", "context", "body_sha256", "transcript",
    }
    if set(metadata) != required:
        raise HandoffError("handoff frontmatter has an unsupported schema")
    if type(metadata["schema_version"]) is not int or metadata["schema_version"] != SCHEMA_VERSION or metadata["type"] != "context-handoff":
        raise HandoffError("handoff schema version or type is unsupported")
    detail = metadata["detail"]
    if not isinstance(detail, str):
        raise HandoffError("handoff detail is invalid")
    if (detail == "topic" and (not isinstance(metadata["topic"], str) or not metadata["topic"].strip())) or (detail != "topic" and metadata["topic"] is not None):
        raise HandoffError("handoff topic is invalid")
    if not isinstance(metadata["created_at"], str):
        raise HandoffError("handoff creation time is invalid")
    try:
        datetime.fromisoformat(metadata["created_at"])
    except ValueError as error:
        raise HandoffError("handoff creation time is invalid") from error
    if not isinstance(metadata["source_client"], str) or metadata["source_client"] not in SOURCE_CLIENTS:
        raise HandoffError("handoff source client is invalid")
    if metadata["task_id"] is not None and not isinstance(metadata["task_id"], str):
        raise HandoffError("handoff task ID is invalid")
    repository = metadata["repository"]
    if not isinstance(repository, dict) or set(repository) != {"branch", "commit"}:
        raise HandoffError("handoff repository provenance is invalid")
    if any(repository[key] is not None and not isinstance(repository[key], str) for key in repository):
        raise HandoffError("handoff repository provenance is invalid")
    files = metadata["files"]
    if not isinstance(files, list) or any(
        not isinstance(item, dict) or set(item) != {"path", "sha256"}
        or not isinstance(item["path"], str)
        or item["sha256"] is not None and (not isinstance(item["sha256"], str) or re.fullmatch(r"[0-9a-f]{64}", item["sha256"]) is None)
        for item in files
    ):
        raise HandoffError("handoff file fingerprints are invalid")
    context = metadata["context"]
    if not isinstance(context, dict):
        raise HandoffError("handoff curated context is invalid")
    validated = validate_curated_context(context, detail)
    if [item["path"] for item in files] != [PurePosixPath(path).as_posix() for path in validated["files"]]:
        raise HandoffError("handoff fingerprints do not match its file references")
    transcript_metadata = metadata["transcript"]
    if (detail == "full") != (transcript_metadata is not None):
        raise HandoffError("only a full handoff must contain a transcript")
    if transcript_metadata is None:
        transcript = None
        expected_body = render_body(validated).encode("utf-8")
    elif (
        isinstance(transcript_metadata, dict)
        and set(transcript_metadata) == {"sha256", "bytes"}
        and isinstance(transcript_metadata["sha256"], str)
        and re.fullmatch(r"[0-9a-f]{64}", transcript_metadata["sha256"])
        and type(transcript_metadata["bytes"]) is int
        and 0 < transcript_metadata["bytes"] <= TRANSCRIPT_MAX_BYTES
    ):
        prefix = render_body(validated).encode("utf-8") + TRANSCRIPT_MARKER
        if not body.startswith(prefix):
            raise HandoffError("handoff transcript marker is missing")
        transcript = body[len(prefix):]
        expected_body = prefix + transcript
        if len(transcript) != transcript_metadata["bytes"] or hashlib.sha256(transcript).hexdigest() != transcript_metadata["sha256"]:
            raise HandoffError("handoff transcript digest does not match")
    else:
        raise HandoffError("handoff transcript metadata is invalid")
    if expected_body != body:
        raise HandoffError("handoff body does not match its curated context")
    digest = hashlib.sha256(body).hexdigest()
    if metadata["body_sha256"] != digest:
        raise HandoffError("handoff body digest does not match")
    return validated, transcript


def save_handoff(
    repository: Path,
    *,
    input_json: str,
    output: Path,
    detail: str,
    topic: str | None,
    task_id: str | None,
    source_client: str,
    transcript_path: Path | None,
) -> dict[str, object]:
    if source_client not in SOURCE_CLIENTS:
        raise HandoffError(f"source client must be one of: {', '.join(sorted(SOURCE_CLIENTS))}")
    if detail == "topic" and (not isinstance(topic, str) or not topic.strip()):
        raise HandoffError("--topic is required when --detail topic")
    if detail != "topic" and topic is not None:
        raise HandoffError("--topic is valid only when --detail topic")
    if detail == "full" and transcript_path is None:
        raise HandoffError("--transcript is required when --detail full")
    if detail != "full" and transcript_path is not None:
        raise HandoffError("--transcript is valid only when --detail full")
    context = validate_curated_context(_read_curated_json(Path(input_json)), detail)
    fingerprints = fingerprint_files(repository, context["files"])  # type: ignore[arg-type]
    _reject_symlink_parents(output, include_leaf=True)
    output = output.resolve()
    if output.suffix != ".md":
        raise HandoffError("handoff output must use a .md extension")
    if output.exists():
        raise HandoffError(f"handoff output already exists: {output}")
    try:
        relative_output = output.relative_to(repository)
    except ValueError:
        relative_output = None
    if relative_output is not None and relative_output.parts[0] in {"memory-bank", "project-brain", ".git", ".agents", ".claude", ".cursor", ".codex"}:
        raise HandoffError("handoff output must be a standalone task artifact, outside memory and policy stores")
    transcript = _read_transcript(transcript_path) if transcript_path else None
    body = render_body(context).encode("utf-8")
    if transcript is not None:
        body += TRANSCRIPT_MARKER + transcript
    metadata: dict[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "type": "context-handoff",
        "created_at": datetime.now(timezone.utc).isoformat(),
        "detail": detail,
        "topic": topic.strip() if isinstance(topic, str) else None,
        "source_client": source_client,
        "task_id": task_id.strip() if isinstance(task_id, str) and task_id.strip() else None,
        "repository": {
            "branch": _git_value(repository, ["branch", "--show-current"]),
            "commit": _git_value(repository, ["rev-parse", "HEAD"]),
        },
        "files": fingerprints,
        "context": context,
        "body_sha256": hashlib.sha256(body).hexdigest(),
        "transcript": (
            {"sha256": hashlib.sha256(transcript).hexdigest(), "bytes": len(transcript)}
            if transcript is not None else None
        ),
    }
    _reject_secrets("handoff", [json.dumps(metadata, ensure_ascii=False), body.decode("utf-8", errors="ignore")])
    output.parent.mkdir(parents=True, exist_ok=True)
    try:
        descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(b"---\n")
            handle.write(json.dumps(metadata, ensure_ascii=False, sort_keys=True).encode("utf-8"))
            handle.write(b"\n---\n")
            handle.write(body)
    except FileExistsError as error:
        raise HandoffError(f"handoff output already exists: {output}") from error
    return {"output": str(output), "detail": detail, "files": len(fingerprints), "transcript": transcript is not None}


def load_handoff(repository: Path, input_path: Path, *, include_transcript: bool = False) -> dict[str, object]:
    metadata, body = _load_markdown(input_path)
    context, transcript = _validate_metadata(metadata, body)
    source = metadata["repository"]
    assert isinstance(source, dict)
    drift: list[dict[str, object]] = []
    current_branch = _git_value(repository, ["branch", "--show-current"])
    current_commit = _git_value(repository, ["rev-parse", "HEAD"])
    if source["branch"] != current_branch:
        drift.append({"kind": "branch", "saved": source["branch"], "current": current_branch})
    if source["commit"] != current_commit:
        drift.append({"kind": "commit", "saved": source["commit"], "current": current_commit})
    current_files = fingerprint_files(repository, context["files"])
    for saved, current_file in zip(metadata["files"], current_files):
        assert isinstance(saved, dict)
        relative = current_file["path"]
        current = current_file["sha256"]
        if current is None or saved["sha256"] != current:
            drift.append({"kind": "file", "path": relative, "saved": saved["sha256"], "current": current})
    result: dict[str, object] = {
        "input": str(input_path.resolve()),
        "metadata": {key: value for key, value in metadata.items() if key != "context"},
        "context": context,
        "drift": drift,
    }
    if include_transcript and transcript is not None:
        result["transcript"] = transcript.decode("utf-8")
    return result
