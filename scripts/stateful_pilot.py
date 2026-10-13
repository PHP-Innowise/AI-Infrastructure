"""Private pre-session excerpt experiment with atomic lexical fallback."""
from __future__ import annotations

import copy
import hashlib
import inspect
import math
import re
import sqlite3
import time
from collections import Counter
from typing import Any

from memory_onnx import MAX_PASSAGES

MAX_OPTIONS = 8
MAX_DOCUMENT = 100_000
FINALIZATION_RESERVE = 1.0


class ProjectionRejected(Exception):
    pass


def membership(capsule: dict[str, Any]) -> tuple[tuple[str, ...], ...]:
    def identity(item: Any) -> str:
        return str(item.get("path") or "episode:" + str(item.get("id")))
    return tuple(tuple(identity(item) for item in capsule.get(layer, []))
                 for layer in ("procedural", "semantic", "episodic", "selected"))


class StatefulExcerptPilot:
    """One score attempt per retrieval; no writes until pack validation passes."""

    def __init__(self, context: Any, runtime: Any, scorer: Any,
                 *, deadline: float | None = None) -> None:
        self.context, self.runtime, self.scorer = context, runtime, scorer
        self.deadline = deadline
        self.stats: Counter[str] = Counter()
        self.original_retrieve = runtime.retrieve
        self.original_context_retrieve = context.retrieve
        self.original_section = runtime.excerpt_section
        self.original_context_section = context.excerpt_section
        self.prepared: dict[str, dict[str, Any]] = {}
        self.lexical_items: dict[str, dict[str, Any]] = {}
        self.connection: sqlite3.Connection | None = None
        self.query = ""
        self.active = False
        self.installed = False

    def install(self) -> None:
        if self.installed:
            return
        if "prepare_excerpts" not in inspect.signature(self.original_retrieve).parameters:
            self.stats["fallback:seam-unavailable"] += 1
            return
        self.runtime.retrieve = self.retrieve
        self.context.retrieve = self.retrieve
        self.runtime.excerpt_section = self.section
        self.context.excerpt_section = self.section
        self.installed = True

    def close(self) -> None:
        if self.installed:
            self.runtime.retrieve = self.original_retrieve
            self.context.retrieve = self.original_context_retrieve
            self.runtime.excerpt_section = self.original_section
            self.context.excerpt_section = self.original_context_section
        self.installed = False
        self.prepared.clear()
        self.active = False

    def section(self, connection: sqlite3.Connection, item: dict[str, Any],
                marked: str, weights: Any = None, *, query: str = "") -> Any:
        if (self.active and connection is self.connection and query == self.query
                and item.get("path") in self.prepared):
            return copy.deepcopy(self.prepared[item["path"]])
        return self.original_section(connection, item, marked, weights, query=query)

    def options(self, connection: sqlite3.Connection, item: dict[str, Any],
                marked: str, weights: dict[str, float], query: str) -> list[dict[str, Any]]:
        lexical = self.original_section(connection, item, marked, weights, query=query)
        if lexical is None:
            return []
        window_limit = self.runtime.SNIPPET_WINDOW_CHARS
        title = str(item.get("title") or "")
        sections = self.runtime._excerpt_sections(marked)
        requested: set[str] = set()
        sources: set[str] | None = None
        if (item.get("selection") in ("path-link", "source-link", *self.runtime.AUTOMATIC_LINK_SELECTIONS)
                and item.get("kind") != "brain-task"):
            requested = {heading for heading in ("sources", "references")
                         if re.search(r"(?<![\w/\\.-])" + heading + r"(?![\w/\\.-])", query, re.I)}
            if not requested:
                try:
                    rows = connection.execute(
                        "SELECT ref_path FROM document_links WHERE path IN (?) AND ref_kind = 'source' ORDER BY ref_path LIMIT ?",
                        (item["path"], self.runtime.GRAPH_ROW_LIMIT + 1),
                    ).fetchall()
                except sqlite3.Error:
                    rows = []
                if rows and len(rows) <= self.runtime.GRAPH_ROW_LIMIT:
                    sources = {str(row[0]) for row in rows}
        substantive = (sources is not None and any(
            self.runtime._excerpt_units(lines)
            and not self.runtime._citation_only_section(heading, lines, sources)
            for heading, lines in sections))
        windows: list[tuple[float, dict[str, Any]]] = []
        for heading, lines in sections:
            plain_heading = " ".join(self.runtime._unmarked(heading).split())
            if requested and plain_heading.casefold() not in requested:
                continue
            if substantive and self.runtime._citation_only_section(heading, lines, sources):
                continue
            units = self.runtime._excerpt_units(lines)
            if not units:
                continue
            section = self.runtime.quoted_section("## " + heading + "\n" + "\n".join(lines), title, {})
            if section is None:
                continue
            texts = [self.runtime.excerpt_window(units, window_limit, weights)]
            block: list[str] = []
            size = 0
            for unit in units:
                length = len(self.runtime._unmarked(unit))
                if block and size + length + 1 > window_limit:
                    texts.append(self.runtime.excerpt_window(block, window_limit, weights))
                    block, size = [], 0
                block.append(unit)
                size += length + 1
            if block:
                texts.append(self.runtime.excerpt_window(block, window_limit, weights))
            score = self.runtime._excerpt_score([heading, *units], weights)[0]
            for text in texts:
                if text:
                    windows.append((score, {**section, "units": [text]}))
        original = {**lexical, "units": [self.runtime.excerpt_window(lexical["units"], window_limit, weights)]}
        ordered = [entry for _, entry in sorted(enumerate(windows), key=lambda pair: (-pair[1][0], pair[0]))[:4]]
        if windows:
            ordered.extend(windows[index] for index in (0, len(windows) // 2, len(windows) - 1))
        result = [original]
        seen = {(original["hash"], tuple(original["units"]))}
        for _, section in ordered:
            key = section["hash"], tuple(section["units"])
            if key not in seen:
                seen.add(key)
                result.append(section)
            if len(result) >= MAX_OPTIONS:
                break
        return result

    def prepare(self, connection: sqlite3.Connection, items: list[dict[str, Any]], query: str) -> None:
        # Never mutate admitted items until every model response is validated.
        self.lexical_items = {item["path"]: copy.deepcopy(item) for item in items}
        try:
            self._prepare(connection, items, query)
        except Exception:
            self.stats["fallback:prepare-error"] += 1
            self.prepared.clear()
            self.active = False

    def _prepare(self, connection: sqlite3.Connection, items: list[dict[str, Any]], query: str) -> None:
        if self.deadline is not None and self.deadline - time.monotonic() <= FINALIZATION_RESERVE:
            self.stats["fallback:deadline-reserve"] += 1
            return
        safe_query = self.context.sanitize_automatic_query(query)
        if not safe_query or len(safe_query) > 1024:
            self.stats["fallback:query-bound"] += 1
            return
        weights = self.runtime.excerpt_weights(connection, query)
        groups: dict[str, list[dict[str, Any]]] = {}
        count = 0
        for item in items:
            if (item.get("layer") == "procedural" or item.get("match") == "distinctive"
                    or item.get("kind") == "brain-history"
                    or item.get("selection") in self.runtime.AUTOMATIC_LINK_SELECTIONS and item.get("kind") == "brain-task"):
                continue
            marked = self.runtime.marked_document(connection, item["path"], query)
            if not marked or len(marked) > MAX_DOCUMENT:
                continue
            options = self.options(connection, item, marked, weights, query)
            if not options:
                continue
            # Preserve option zero and the item's already admitted budget. An
            # impossible alternative must not displace a feasible scored one
            # or consume the shared worker's bounded passage allowance.
            admitted = [options[0]]
            title = str(item.get("title") or "")
            for section in options[1:]:
                safe_text = " ".join(self.context.sanitize_automatic_query(section["units"][0]).split())
                if (not safe_text or len(safe_text) > self.runtime.SNIPPET_WINDOW_CHARS
                        or self.runtime._estimate_tokens(title + safe_text) > item["estimated_tokens"]):
                    self.stats["filtered-options"] += 1
                    continue
                admitted.append({**section, "units": [safe_text]})
            options = admitted[:MAX_PASSAGES - count]
            if len(options) > 1:
                groups[item["path"]] = options
                count += len(options)
        if not groups:
            self.stats["no-alternatives"] += 1
            return
        passages = [section["units"][0] for options in groups.values() for section in options]
        safe = [" ".join(self.context.sanitize_automatic_query(text).split()) for text in passages]
        if any(not text for text in safe):
            self.stats["fallback:screened-input"] += 1
            return
        self.stats["worker-calls"] += 1
        deadline = None if self.deadline is None else self.deadline - FINALIZATION_RESERVE
        result = self.scorer.score(safe_query, safe, deadline=deadline)
        values = result.scores
        if (values is None or len(values) != len(passages)
                or any(isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) for value in values)):
            self.stats["fallback:" + (result.reason or "invalid-output")] += 1
            return
        prepared: dict[str, dict[str, Any]] = {}
        offset = 0
        for doc_ref, options in groups.items():
            chosen = max(range(len(options)), key=lambda index: values[offset + index])
            safe_text = safe[offset + chosen]
            offset += len(options)
            if chosen:
                prepared[doc_ref] = {**options[chosen], "units": [safe_text]}
        self.prepared = prepared
        self.active = bool(prepared)
        changed: dict[str, dict[str, Any]] = {}
        for item in items:
            if item["path"] not in prepared:
                continue
            value = copy.deepcopy(item)
            value["snippet"] = prepared[item["path"]]["units"][0]
            value["estimated_tokens"] = self.runtime._estimate_tokens(str(value.get("title") or "") + value["snippet"])
            if value["estimated_tokens"] > item["estimated_tokens"]:
                self.stats["fallback:snippet-budget"] += 1
                self.prepared.clear()
                self.active = False
                return
            value["_excerpt_identity"] = self.runtime.delivery_identity(connection, value, query)
            changed[item["path"]] = value
        for item in items:
            if item["path"] in changed:
                item.update(changed[item["path"]])
        self.stats["proposed-excerpts"] += len(changed)

    def restore_lexical(self, result: dict[str, Any]) -> dict[str, Any]:
        output = copy.deepcopy(result)
        for key in ("procedural", "semantic", "episodic", "selected"):
            for item in output.get(key, []):
                original = self.lexical_items.get(item.get("path"))
                if original is not None:
                    item["snippet"] = original.get("snippet", "")
                    item["estimated_tokens"] = original["estimated_tokens"]
        for group in output.get("categories", {}).values():
            for item in group:
                original = self.lexical_items.get(item.get("path"))
                if original is not None:
                    item["snippet"] = original.get("snippet", "")
                    item["estimated_tokens"] = original["estimated_tokens"]
        return output

    def retrieve(self, connection: sqlite3.Connection, repository: Any, query: str,
                 *args: Any, **kwargs: Any) -> Any:
        self.connection, self.query = connection, query
        self.prepared.clear()
        self.lexical_items.clear()
        self.active = False
        original_pack = kwargs.get("pack")

        def guarded_pack(result: dict[str, Any]) -> dict[str, Any]:
            if not self.active or original_pack is None:
                return original_pack(result) if original_pack else result
            lexical_input = self.restore_lexical(result)
            self.active = False
            try:
                baseline = original_pack(lexical_input)
            finally:
                self.active = True
            proposed = original_pack(result)
            if membership(baseline) != membership(proposed):
                raise ProjectionRejected()
            return proposed

        try:
            result = self.original_retrieve(connection, repository, query, *args, **{
                **kwargs, "pack": guarded_pack, "prepare_excerpts": self.prepare})
            delivered = {item.get("path") for layer in ("semantic", "episodic")
                         for item in result.get(layer, [])}
            self.stats["changed-excerpts"] += len(delivered.intersection(self.prepared))
            return result
        except ProjectionRejected:
            self.stats["fallback:pack-change"] += 1
            self.prepared.clear()
            self.active = False
            return self.original_retrieve(connection, repository, query, *args, **kwargs)
