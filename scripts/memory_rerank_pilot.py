#!/usr/bin/env python3
"""Compare baseline candidates with a deterministic document feature reranker.

Read-only pilot, standard library only. It loads a local context index, asks
the runtime for its normal candidate list, and reports what a path/title
feature reranker would put in the same top-k. It deliberately does not change
the runtime: the pilot exists to decide whether such a reranker should be
integrated, and its weights are explicit.

The feature reranker is not a model. It rewards a candidate whose path or
title contains the query's informative words; the base BM25/authority score
still dominates. The output is a comparison, not a quality claim: a useful
answer needs judged queries, and the pilot never invents judgments.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sqlite3
import sys
from pathlib import Path
from typing import Any, Iterable, Optional


DEFAULT_LIMIT = 3
CANDIDATE_WINDOW = 100
PATH_WEIGHT = 0.15
TITLE_WEIGHT = 0.10
TOKEN = re.compile(r"[a-z0-9]{3,}")
STOPWORDS = frozenset({
    "the", "and", "for", "with", "from", "that", "this", "how", "does",
    "what", "where", "when", "why", "are", "was", "were", "has", "have",
    "had", "you", "your", "into", "over", "under", "about", "its", "not",
})


PATH_EXTENSIONS = frozenset({
    "php", "json", "yaml", "yml", "html", "htm", "xml", "sql", "java",
    "python", "javascript", "typescript", "markdown", "toml", "ini",
})


def _tokens(value: object) -> set[str]:
    return {
        token for token in TOKEN.findall(str(value or "").casefold())
        if token not in STOPWORDS
    }


def _path_tokens(value: object) -> set[str]:
    """Path words, not the extension every file in the directory shares."""
    return {
        token for token in _tokens(value)
        if token not in PATH_EXTENSIONS
    }


def feature_score(item: dict[str, Any], query: str) -> float:
    """A bounded multiplier from path and title overlap with the query."""
    wanted = _tokens(query)
    if not wanted:
        return 1.0
    path_hits = len(wanted & _path_tokens(item.get("path")))
    title_hits = len(wanted & _tokens(item.get("title")))
    return 1.0 + PATH_WEIGHT * min(path_hits, 3) + TITLE_WEIGHT * min(title_hits, 2)


def rerank(
    candidates: Iterable[dict[str, Any]], query: str, limit: int = DEFAULT_LIMIT
) -> list[dict[str, Any]]:
    """Order candidates by base score multiplied by the explicit features."""
    ranked = list(candidates)
    scored = [
        (float(item.get("adjusted_score") or 0.0) * feature_score(item, query), -index, item)
        for index, item in enumerate(ranked)
    ]
    scored.sort(key=lambda row: (-row[0], row[1]))
    return [item for _, _, item in scored[: max(0, limit)]]


def metrics(
    selected: Iterable[dict[str, Any]], judgments: dict[str, int]
) -> dict[str, Any]:
    """Useful/noise counts and useful-path recall for one top-k."""
    useful = {path for path, grade in judgments.items() if grade > 0}
    chosen = {str(item.get("path")) for item in selected}
    return {
        "useful": len(chosen & useful),
        "noise": len(chosen - useful),
        "recall": round(len(chosen & useful) / len(useful), 4) if useful else None,
        "paths": sorted(chosen),
    }


def load_runtime(scripts: Path) -> Any:
    source = scripts.resolve() / "context_retrieval.py"
    if not source.is_file():
        raise ValueError(f"runtime unavailable: {source}")
    sys.path.insert(0, str(source.parent))
    spec = importlib.util.spec_from_file_location("memory_rerank_runtime", source)
    if spec is None or spec.loader is None:
        raise ValueError("runtime unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_queries(path: Path) -> list[dict[str, str]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(value, dict):
        items = [{"id": str(key), "query": str(query)} for key, query in value.items()]
    elif isinstance(value, list):
        items = []
        for index, entry in enumerate(value):
            if not isinstance(entry, dict):
                raise ValueError(f"query {index} is not an object")
            query = entry.get("query", entry.get("prompt"))
            if not isinstance(query, str):
                raise ValueError(f"query {index} lacks a string 'query' or 'prompt'")
            items.append({
                "id": str(entry.get("id", index)),
                "query": query,
            })
    else:
        raise ValueError("queries must be an object or an array")
    if not items:
        raise ValueError("queries are empty")
    return items


def load_judgments(path: Optional[Path]) -> dict[str, dict[str, int]]:
    if path is None:
        return {}
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("judgments must be an object")
    result: dict[str, dict[str, int]] = {}
    for prompt_id, grades in value.items():
        if not isinstance(grades, dict):
            raise ValueError(f"judgments for {prompt_id!r} must be an object")
        result[str(prompt_id)] = {
            str(doc): int(grade) for doc, grade in grades.items()
            if isinstance(grade, int) and not isinstance(grade, bool) and grade >= 0
        }
    return result


def evaluate(
    runtime: Any,
    connection: sqlite3.Connection,
    queries: list[dict[str, str]],
    judgments: dict[str, dict[str, int]],
    limit: int,
) -> dict[str, Any]:
    rows = []
    baseline_useful = 0
    reranked_useful = 0
    for entry in queries:
        query = entry["query"]
        candidates, _diagnostics = runtime._candidates(
            connection, query, limit=CANDIDATE_WINDOW
        )
        baseline = candidates[:limit]
        reranked = rerank(candidates, query, limit)
        grades = judgments.get(entry["id"], {})
        baseline_metrics = metrics(baseline, grades)
        reranked_metrics = metrics(reranked, grades)
        baseline_useful += baseline_metrics["useful"]
        reranked_useful += reranked_metrics["useful"]
        rows.append({
            "id": entry["id"],
            "query": query,
            "baseline": baseline_metrics,
            "reranked": reranked_metrics,
            "changed": baseline_metrics["paths"] != reranked_metrics["paths"],
        })
    return {
        "queries": len(rows),
        "limit": limit,
        "weights": {"path": PATH_WEIGHT, "title": TITLE_WEIGHT},
        "baseline_useful": baseline_useful,
        "reranked_useful": reranked_useful,
        "rows": rows,
    }


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--runtime", type=Path,
        default=Path(__file__).resolve().parent.parent / "PHP Core/memory-bank/scripts",
        help="directory containing context_retrieval.py",
    )
    parser.add_argument("--database", type=Path, required=True, help="local context.db")
    parser.add_argument("--queries", type=Path, required=True, help="JSON queries")
    parser.add_argument("--judgments", type=Path, help="optional JSON grades")
    parser.add_argument("--limit", type=int, default=DEFAULT_LIMIT)
    parser.add_argument("--json", action="store_true", help="print JSON only")
    arguments = parser.parse_args(argv)
    try:
        runtime = load_runtime(arguments.runtime)
        queries = load_queries(arguments.queries)
        judgments = load_judgments(arguments.judgments)
        database = arguments.database.resolve()
        if not database.is_file():
            raise ValueError(f"database unavailable: {database}")
        connection = sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)
        connection.row_factory = sqlite3.Row
        try:
            report = evaluate(runtime, connection, queries, judgments, max(1, arguments.limit))
        finally:
            connection.close()
    except (OSError, ValueError, sqlite3.Error, json.JSONDecodeError) as error:
        print(f"memory_rerank_pilot: {error}", file=sys.stderr)
        return 2
    if arguments.json:
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    for row in report["rows"]:
        print(
            f"{row['id']}: baseline {row['baseline']['useful']} useful, "
            f"reranked {row['reranked']['useful']} useful"
            + (" (changed)" if row["changed"] else "")
        )
    print(
        f"{report['queries']} queries: useful {report['baseline_useful']} -> "
        f"{report['reranked_useful']} at k={report['limit']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
