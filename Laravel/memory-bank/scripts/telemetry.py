#!/usr/bin/env python3
"""Privacy-safe, disabled-by-default metadata telemetry adapter."""

from __future__ import annotations

import json
import math
import os
import tempfile
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional


class TelemetryError(Exception):
    """A sanitized telemetry contract error."""


ALLOWED_INPUT_FIELDS = {
    "provider",
    "model",
    "operation",
    "input_tokens",
    "output_tokens",
    "context_tokens",
    "cached_input_tokens",
    "cost_usd",
    "ttfr_ms",
    "task_id",
}
TOKEN_FIELDS = (
    "input_tokens",
    "output_tokens",
    "context_tokens",
    "cached_input_tokens",
)
NUMBER_FIELDS = (
    "cost_usd",
    "ttfr_ms",
)
METRIC_FIELDS = (*TOKEN_FIELDS, *NUMBER_FIELDS)


def _load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise TelemetryError("Telemetry configuration is unavailable") from error
    if not isinstance(value, dict):
        raise TelemetryError("Telemetry configuration must be an object")
    return value


def _atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(
                value,
                handle,
                ensure_ascii=False,
                sort_keys=True,
                allow_nan=False,
            )
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def write_metadata_event(
    repository: Path, metadata: dict[str, Any]
) -> Optional[Path]:
    """Write one metadata-only event, or return None when telemetry is disabled."""
    config = _load_json(repository / "project-brain/config/telemetry.json")
    runtime = _load_json(repository / "project-brain/config/runtime.json")
    if not config.get("enabled") or not runtime.get("telemetry_enabled"):
        return None
    if not config.get("metadata_only"):
        raise TelemetryError("Telemetry must remain metadata-only")
    prohibited = set(config.get("prohibited_fields", []))
    supplied = set(metadata)
    if supplied & prohibited or supplied - ALLOWED_INPUT_FIELDS:
        raise TelemetryError("Telemetry contains prohibited or unknown fields")

    provider = metadata.get("provider")
    operation = metadata.get("operation")
    if not isinstance(provider, str) or not provider.strip():
        raise TelemetryError("Telemetry provider is required")
    if not isinstance(operation, str) or not operation.strip():
        raise TelemetryError("Telemetry operation is required")
    model = metadata.get("model")
    if model is not None and (
        not isinstance(model, str) or not model.strip()
    ):
        raise TelemetryError("Telemetry model must be a string or N/A")
    task_id = metadata.get("task_id")
    canonical_task_id = None
    if task_id is not None:
        try:
            parsed_task_id = uuid.UUID(task_id)
        except (AttributeError, TypeError, ValueError) as error:
            raise TelemetryError("Telemetry task_id must be UUIDv4 or N/A") from error
        if parsed_task_id.version != 4:
            raise TelemetryError("Telemetry task_id must be UUIDv4 or N/A")
        canonical_task_id = str(parsed_task_id)

    metrics = {field: metadata.get(field) for field in METRIC_FIELDS}
    for field in TOKEN_FIELDS:
        value = metrics[field]
        if value is not None and (
            not isinstance(value, int) or isinstance(value, bool) or value < 0
        ):
            raise TelemetryError(
                f"Telemetry metric {field} must be a nonnegative integer or N/A"
            )
    for field in NUMBER_FIELDS:
        value = metrics[field]
        if value is not None and (
            not isinstance(value, (int, float))
            or isinstance(value, bool)
            or not math.isfinite(value)
            or value < 0
        ):
            raise TelemetryError(
                f"Telemetry metric {field} must be a finite nonnegative number or N/A"
            )
    event = {
        "schema_version": 1,
        "id": str(uuid.uuid4()),
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "provider": provider.strip(),
        "model": (
            model.strip() if isinstance(model, str) else None
        ),
        "operation": operation.strip(),
        **metrics,
        "availability": (
            "measured" if all(value is not None for value in metrics.values()) else "N/A"
        ),
        "task_id": canonical_task_id,
    }
    destination = (
        repository
        / "memory-bank/local/telemetry"
        / f"{event['id']}.json"
    )
    _atomic_json(destination, event)
    return destination
