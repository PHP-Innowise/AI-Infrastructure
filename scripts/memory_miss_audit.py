#!/usr/bin/env python3
"""Read-only, opt-in miss tracing for ``scripts/memory_eval.py``.

This is deliberately a subprocess: its environment is the one that ran the
refresh, including MEMORY_EVAL_CLOCK/PYTHONPATH.  It emits only caller supplied
judgment paths and fixed stage labels; query, passages, document text and
unjudged paths never leave the ephemeral corpus.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any, Iterable, Optional, Sequence


MANIFEST_PATH = re.compile(
    r"(?:memory-bank/local|project-brain/control)/retrieval-manifests/[0-9a-f-]{36}\.json"
)
STAGES = frozenset({
    "absent-source", "not-indexed", "no-fts-candidate", "window-limit",
    "relevance-drop", "floor-drop", "runtime-exclusion", "rank-loss",
    "budget-loss", "capsule-loss", "selected", "selected-answer-not-in-text", "unknown", "unknown-history",
    "unknown-query-withheld",
    "history-event-limit", "history-admission-drop", "automatic-link-limit",
})
ANSWER_STATES = frozenset({
    "not-applicable", "unknown", "not-in-source", "not-in-index", "not-in-delivery",
    "partial-index", "partial-delivery", "delivered",
})
RUNTIME_REASONS = frozenset({
    "privacy", "owner", "authority", "lifecycle", "stale", "map-unverifiable",
    "map-drift", "source-missing", "working-task", "host-loaded",
    "promoted-to-chunk", "conflict-unavailable",
    # A durable record, finding or changelog naming only other tickets than
    # the request's; a Brain record that failed validation; a context-handoff
    # snapshot, which only context-load returns. Unmapped, a withheld
    # changelog read as an unknown stage.
    "ticket-mismatch", "invalid", "explicit-context-only",
})
# Path links and file links are both automatic links with a cap of their own.
AUTOMATIC_LINK_REASONS = frozenset({"file-link-limit", "path-link-limit"})
RANK_REASONS = frozenset({
    "layer-limit", "episodic-layer", "skill-subfile", "host-listed",
    "delivered-this-session",
})
REFRESH_RETRIEVAL_LIMIT = 3


def _safe_relative(value: object) -> Optional[str]:
    if not isinstance(value, str) or not value or "\\" in value:
        return None
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts:
        return None
    return path.as_posix()


def _source_paths(corpus: Path, path: str) -> list[tuple[str, Path]]:
    """Judgment aliases only; do not discover or expose any other path."""
    candidates = [(path, corpus / PurePosixPath(path))]
    prefix = ".agents/skills/"
    if path.startswith(prefix):
        suffix = path[len(prefix):]
        candidates.extend(
            (tool + suffix, corpus / PurePosixPath(tool + suffix))
            for tool in (".claude/skills/", ".cursor/skills/", ".codex/skills/")
        )
    root = corpus.resolve()
    safe = []
    for indexed, target in candidates:
        resolved = target.resolve()
        if target.is_file() and not target.is_symlink() and resolved.is_relative_to(root):
            safe.append((indexed, target))
    return safe


def _load_runtime(corpus: Path) -> Any:
    scripts = corpus / "memory-bank" / "scripts"
    source = scripts / "context_retrieval.py"
    if not source.is_file():
        raise ValueError("runtime-unavailable")
    sys.path.insert(0, str(scripts))
    try:
        spec = importlib.util.spec_from_file_location("memory_miss_runtime", source)
        if spec is None or spec.loader is None:
            raise ValueError("runtime-unavailable")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        from validate import sanitize_automatic_query
        # Bind the function while this corpus's import path is active;
        # history tracing must not depend on another caller's module cache.
        module.miss_audit_sanitizer = sanitize_automatic_query
        return module
    finally:
        sys.path.pop(0)


def _read_manifest(corpus: Path, reference: object, runtime: Any) -> dict[str, Any]:
    if not isinstance(reference, str) or not MANIFEST_PATH.fullmatch(reference):
        raise ValueError("manifest-unavailable")
    path = (corpus / PurePosixPath(reference)).resolve()
    if not path.is_file() or not path.is_relative_to(corpus.resolve()):
        raise ValueError("manifest-unavailable")
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
        runtime.validate_schema_file(corpus, "retrieval-manifest.schema.json", manifest)
    except Exception as error:
        raise ValueError("manifest-invalid") from error
    if not isinstance(manifest, dict) or not isinstance(manifest.get("query"), str):
        raise ValueError("manifest-invalid")
    return manifest


def _informative_tokens(connection: sqlite3.Connection, runtime: Any, query: str) -> list[str]:
    """Read-only equivalent of the runtime's query vocabulary.

    The runtime expands an identifier's words before filtering; the audit must
    ask the same question or its stages describe a different retrieval than
    the one the capsule used.
    """
    tokens = runtime.expand_query_tokens(connection, runtime.query_tokens(query))
    total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0]
    if not total:
        return tokens
    frequencies: dict[str, int] = {}
    for token in tokens:
        frequencies[token] = connection.execute(
            "SELECT COUNT(*) FROM documents WHERE documents MATCH ?", (f'"{token}"',)
        ).fetchone()[0]
    matched = [(token, frequencies[token]) for token in tokens if frequencies[token]]
    informative = [
        token for token, count in matched
        if count <= total * runtime.STOPWORD_DOCUMENT_RATIO
    ]
    return informative or [token for token, _count in matched] or tokens


def _window_rows(connection: sqlite3.Connection, runtime: Any, tokens: list[str], limit: int) -> list[sqlite3.Row]:
    expression = " OR ".join(f'"{token}"' for token in tokens)
    return connection.execute(
        """
        SELECT d.path, d.layer, d.kind, d.title, m.authority, m.confidence,
               m.updated_at, bm25(documents, ?, ?, ?, ?, ?, ?) AS score
        FROM documents AS d JOIN document_metadata AS m ON m.path = d.path
        WHERE documents MATCH ? AND d.kind != ? ORDER BY score, d.path LIMIT ?
        """,
        (*runtime.BM25_WEIGHTS, expression, runtime.HISTORY_KIND, limit),
    ).fetchall()


def _stage_for_indexed(
    connection: sqlite3.Connection, runtime: Any, query: str, indexed_path: str, limit: int
) -> str:
    kind = connection.execute("SELECT kind FROM documents WHERE path = ?", (indexed_path,)).fetchone()
    if kind is not None and kind[0] == runtime.HISTORY_KIND:
        return "unknown-history"
    tokens = _informative_tokens(connection, runtime, query)
    evidence = runtime.evidence_tokens(tokens)
    if not evidence:
        return "unknown"
    rows = _window_rows(connection, runtime, tokens, limit)
    target = next((row for row in rows if row["path"] == indexed_path), None)
    expression = " OR ".join(f'"{token}"' for token in tokens)
    lexical = connection.execute(
        "SELECT 1 FROM documents AS d WHERE documents MATCH ? AND d.kind != ? AND d.path = ? LIMIT 1",
        (expression, runtime.HISTORY_KIND, indexed_path),
    ).fetchone()
    if target is None:
        return "window-limit" if lexical is not None else "no-fts-candidate"
    coverage, distinctive = runtime.token_coverage(connection, evidence)
    minimum = runtime.required_coverage(evidence)
    relevant: list[dict[str, Any]] = []
    target_relevant = False
    for row in rows:
        accepted = runtime.is_relevant(
            row["path"], coverage, distinctive, minimum, str(row["title"] or ""),
            anchored=row["layer"] != "procedural",
        )
        if row["path"] == indexed_path:
            target_relevant = accepted
        if not accepted:
            continue
        item = dict(row)
        item["adjusted_score"] = max(0.0, -float(item["score"])) * runtime.ranking_weight(
            item["authority"], item["confidence"], item["updated_at"]
        )
        relevant.append(item)
    if not target_relevant:
        return "relevance-drop"
    best_by_layer: dict[str, float] = {}
    for item in sorted(relevant, key=lambda value: (-value["adjusted_score"], value["score"], value["path"])):
        best_by_layer.setdefault(item["layer"], item["adjusted_score"])
    target_item = next(item for item in relevant if item["path"] == indexed_path)
    if target_item["adjusted_score"] < runtime.RELATIVE_SCORE_FLOOR * best_by_layer[target_item["layer"]]:
        return "floor-drop"
    return "unknown"


def _flat(value: str) -> str:
    return " ".join(value.split()).casefold()


def _answer_flat(value: str) -> str:
    """Flatten text for answer presence, excluding Markdown heading lines.

    A labelled changelog passage starts with `# Changelog` and its section
    heading, while the capsule deliberately quotes the entry body. The
    answer is present even when the presentation heading is not; treating the
    heading as required text made six selected entries look undelivered.
    """
    body = "\n".join(
        line for line in value.splitlines() if not line.lstrip().startswith("#")
    )
    return _flat(body or value)


def _history_stage(corpus: Path, connection: sqlite3.Connection, runtime: Any,
                   path: str, text: Optional[str]) -> str:
    if text is None:
        return "unknown-history"
    # The manifest holds the distilled main query. Recorded history instead
    # uses the whole screened prompt; never substitute one for the other.
    screened = runtime.miss_audit_sanitizer(text)
    try:
        history = runtime.history_query(connection, screened)
        events, episodes = runtime._history_candidates(connection, history) if history else ([], [])
    except (OSError, sqlite3.Error):
        return "unknown-history"
    if not any(item["path"] == path for item in events):
        return "history-admission-drop"
    try:
        winner, excluded = runtime._best_history_event(
            corpus, events, runtime.load_config(corpus), connection=connection
        )
    except (OSError, sqlite3.Error):
        return "unknown-history"
    except Exception as error:
        # The evaluation corpus is a copy without the project's Git context;
        # `_best_history_event` asks Git for the source policy and raises
        # SourceGitError. A missing context is an unknown stage, not a reason
        # to lose every judged path in the prompt.
        source_error = getattr(
            getattr(runtime, "workspace_roots", None), "SourceGitError", None
        )
        if source_error is not None and isinstance(error, source_error):
            return "unknown-history"
        raise
    if any(item.get("path") == path for item in excluded):
        return "runtime-exclusion"
    if episodes and (not winner or episodes[0][0] > winner[0]["adjusted_score"]):
        return "history-event-limit"
    if winner and winner[0]["path"] != path:
        return "history-event-limit"
    if not winner:
        # No event survived admission and the target was not one of the
        # recorded exclusions: the event slot is empty for this query.
        return "history-admission-drop"
    return "unknown-history"


def _answer_status(
    connection: sqlite3.Connection, indexed: str, sources: list[tuple[str, Path]],
    passages: list[str], capsule_text: str,
) -> tuple[str, dict[str, int]]:
    """Fixed answer-presence label; no source or passage leaves this process."""
    if not passages:
        return "not-applicable", {"eligible": 0, "indexed": 0, "delivered": 0}
    try:
        source_text = _flat(sources[0][1].read_text(encoding="utf-8", errors="replace"))
        content = connection.execute("SELECT content FROM documents WHERE path = ?", (indexed,)).fetchone()
        indexed_text = _flat(str(content[0] or "")) if content is not None else ""
    except (OSError, sqlite3.Error):
        return "unknown", {"eligible": 0, "indexed": 0, "delivered": 0}
    wanted = sorted({_flat(value) for value in passages if isinstance(value, str) and _flat(value)})
    bodies = {
        _flat(value): _answer_flat(value) or _flat(value)
        for value in passages if isinstance(value, str) and _flat(value)
    }
    if not wanted:
        return "not-applicable", {"eligible": 0, "indexed": 0, "delivered": 0}
    eligible = {value for value in wanted if value in source_text}
    indexed = {value for value in eligible if value in indexed_text}
    capsule_body = _answer_flat(capsule_text)
    delivered = {
        value for value in indexed
        if bodies.get(value) and bodies[value] in capsule_body
    }
    counts = {"eligible": len(eligible), "indexed": len(indexed), "delivered": len(delivered)}
    if not eligible:
        return "not-in-source", counts
    if not indexed:
        return "not-in-index", counts
    if len(indexed) < len(eligible):
        return "partial-index", counts
    if not delivered:
        return "not-in-delivery", counts
    if len(delivered) < len(eligible):
        return "partial-delivery", counts
    return "delivered", counts


def _seed_git(corpus: Path, when: Optional[str]) -> bool:
    """Seed a disposable Git snapshot so history admission can be audited.

    The evaluation corpus is copied without `.git`; `_best_history_event`
    asks Git for the source policy and otherwise fails. A temporary repository
    whose files are all tracked/clean is enough for that diagnostic, but it is
    not the project's real history: the caller must treat its result as an
    audit fallback, never as a runtime claim. Returns whether `.git` was
    created here (and therefore must be removed).
    """
    if (corpus / ".git").exists():
        return False
    environment = {
        key: value for key, value in os.environ.items()
        if not key.startswith("GIT_")
    }
    environment.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
    )
    if when:
        environment.update(GIT_AUTHOR_DATE=when, GIT_COMMITTER_DATE=when)
    try:
        subprocess.run(["git", "init", "-q"], cwd=corpus, env=environment, check=True)
        subprocess.run(["git", "add", "-A"], cwd=corpus, env=environment, check=True)
        subprocess.run(
            ["git", "-c", "user.name=memory-eval", "-c", "user.email=memory-eval@example.invalid",
             "commit", "-q", "-m", "memory-eval audit snapshot"],
            cwd=corpus, env=environment, check=True,
        )
    except (OSError, subprocess.SubprocessError):
        shutil.rmtree(corpus / ".git", ignore_errors=True)
        return False
    return True


def audit(
    corpus: Path, manifest_reference: object, known_paths: Iterable[object], *,
    query_withheld: bool = False, answer_passages: Optional[dict[str, list[str]]] = None,
    capsule_text: str = "", history_text: Optional[str] = None,
    seed_git: bool = False,
) -> dict[str, Any]:
    paths = [path for value in known_paths if (path := _safe_relative(value)) is not None]
    if query_withheld:
        return {"status": "unknown", "items": [{"path": path, "stage": "unknown-query-withheld", "answer": "unknown", "answer_counts": {"eligible": 0, "indexed": 0, "delivered": 0}} for path in paths]}
    try:
        runtime = _load_runtime(corpus)
        manifest = _read_manifest(corpus, manifest_reference, runtime)
        database = corpus / "memory-bank" / "local" / "context.db"
        if not database.is_file() or database.is_symlink() or not database.resolve().is_relative_to(corpus.resolve()):
            raise ValueError("index-unavailable")
        connection = sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)
        connection.row_factory = sqlite3.Row
        seeded_git = bool(
            seed_git and _seed_git(corpus, manifest.get("created_at"))
        )
    except Exception:
        return {"status": "unknown", "items": [{"path": path, "stage": "unknown", "answer": "unknown", "answer_counts": {"eligible": 0, "indexed": 0, "delivered": 0}} for path in paths]}
    try:
        # memory_eval passes this explicit refresh --limit, so its candidate
        # window stays in lockstep with the replay rather than relying on a
        # runtime default that could change independently.
        limit = max(20, REFRESH_RETRIEVAL_LIMIT * 10)
        excluded = {
            str(item.get("path")): str(item.get("reason"))
            for item in manifest.get("excluded", []) if isinstance(item, dict)
        }
        selected = {
            str(item.get("path")) for item in manifest.get("selected", []) if isinstance(item, dict)
        }
        items = []
        answer_passages = answer_passages or {}
        for path in paths:
            try:
                sources = _source_paths(corpus, path)
                if not sources:
                    stage, indexed = "absent-source", path
                else:
                    indexed = next((name for name, _source in sources if connection.execute(
                        "SELECT 1 FROM documents WHERE path = ?", (name,)
                    ).fetchone() is not None), sources[0][0])
                    if connection.execute("SELECT 1 FROM documents WHERE path = ?", (indexed,)).fetchone() is None:
                        stage = "not-indexed"
                    elif indexed in selected:
                        stage = "selected"
                    elif indexed in excluded:
                        reason = excluded[indexed]
                        stage = (
                            "budget-loss" if reason in {"budget", "hard-budget"}
                            else "automatic-link-limit" if reason in AUTOMATIC_LINK_REASONS
                            else "capsule-loss" if reason == "capsule-limit"
                            else "rank-loss" if reason in RANK_REASONS
                            else "runtime-exclusion" if reason in RUNTIME_REASONS
                            else "unknown"
                        )
                    else:
                        stage = _stage_for_indexed(connection, runtime, manifest["query"], indexed, limit)
                        if stage == "unknown-history":
                            stage = _history_stage(corpus, connection, runtime, indexed, history_text)
                answer = "not-applicable"
                answer_counts = {"eligible": 0, "indexed": 0, "delivered": 0}
                passages = answer_passages.get(path, [])
                history = False
                if sources:
                    kind_row = connection.execute(
                        "SELECT kind FROM documents WHERE path = ?", (indexed,)
                    ).fetchone()
                    history = bool(kind_row is not None and kind_row[0] == runtime.HISTORY_KIND)
                if sources and connection.execute("SELECT 1 FROM documents WHERE path = ?", (indexed,)).fetchone() is not None:
                    answer, answer_counts = _answer_status(connection, indexed, sources, passages, capsule_text)
                    if stage == "selected" and answer in {"not-in-delivery", "partial-delivery"}:
                        stage = "selected-answer-not-in-text"
                elif passages:
                    answer = "unknown"
                items.append({
                    "path": path, "stage": stage, "answer": answer,
                    "answer_counts": answer_counts, "history": history,
                })
            except Exception:
                # One path's unavailable history context must not erase
                # the stages already established for the prompt.
                items.append({
                    "path": path, "stage": "unknown", "answer": "unknown",
                    "answer_counts": {"eligible": 0, "indexed": 0, "delivered": 0},
                    "history": False,
                })
        return {"status": "ok", "items": items}
    except Exception:
        return {"status": "unknown", "items": [{"path": path, "stage": "unknown", "answer": "unknown", "answer_counts": {"eligible": 0, "indexed": 0, "delivered": 0}} for path in paths]}
    finally:
        connection.close()
        if seeded_git:
            shutil.rmtree(corpus / ".git", ignore_errors=True)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--paths-json", required=True)
    parser.add_argument("--query-withheld", action="store_true")
    parser.add_argument("--audit-git", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        paths = json.loads(arguments.paths_json)
        if not isinstance(paths, list):
            raise ValueError
    except ValueError:
        return 2
    try:
        private = json.load(sys.stdin)
        answers = private.get("answer_passages") if isinstance(private, dict) else {}
        text = private.get("capsule_text") if isinstance(private, dict) else ""
        history = private.get("history_text") if isinstance(private, dict) else None
        if not isinstance(answers, dict) or not isinstance(text, str):
            raise ValueError
        if history is not None and not isinstance(history, str):
            raise ValueError
    except (ValueError, json.JSONDecodeError):
        return 2
    print(json.dumps(
        audit(arguments.corpus.resolve(), arguments.manifest, paths,
              query_withheld=arguments.query_withheld, answer_passages=answers,
              capsule_text=text, history_text=history, seed_git=arguments.audit_git),
        sort_keys=True,
    ))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
