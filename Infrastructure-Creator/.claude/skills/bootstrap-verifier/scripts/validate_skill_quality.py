#!/usr/bin/env python3
"""Deterministically validate generated skills against a semantic JSON plan."""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import os
import re
import shlex
import sys
from collections import Counter, defaultdict, deque
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable
from urllib.parse import urlparse

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from analyze_commands import (
    CODE_HOST_EXECUTABLES,
    CommandAnalysisError,
    CommandAnalyzer,
    RUNNER_EXECUTABLES,
)
from validate_flow_contracts import validate_plan_graph

DEFAULT_REGISTRY = (
    SCRIPT_DIR.parent.parent
    / "skill-forge"
    / "references"
    / "candidate-registry.json"
)
RUNTIME_CONTRACT = (
    SCRIPT_DIR.parent.parent / "memory-seed" / "assets" / "runtime-contract.json"
)

LEGACY_PLAN_FIELDS = (
    "schema_version",
    "catalog_version",
    "target_root",
    "profile",
    "evidence",
    "skills",
    "rejected_candidates",
)
SCHEMA_1_2_PLAN_FIELDS = LEGACY_PLAN_FIELDS + (
    "critical_invariants",
    "flow_contracts",
)
REQUIRED_SKILL_FIELDS = (
    "name",
    "category",
    "kind",
    "phase",
    "necessity_rationale",
    "selection_gate",
    "triggers",
    "evidence_ids",
    "source_paths",
    "owned_scope",
    "excluded_scope",
    "required_procedure_roles",
    "decision_points",
    "verification",
    "output_contract",
    "failure_handling",
    "related_skills",
    "nearest_siblings",
    "writes",
)
SCHEMA_1_2_SKILL_FIELDS = REQUIRED_SKILL_FIELDS + (
    "capability",
    "procedure_steps",
    "integration_safety",
    "path_contracts",
    "evidence_anchors",
    "routing_cases",
)
SECTION_ALIASES = {
    "purpose": ("purpose",),
    "inputs": ("project evidence", "inputs", "project evidence / inputs"),
    "procedure": ("procedure", "process", "procedure / process"),
    "verification": ("verification",),
    "outputs": ("outputs", "output contract"),
    "failure": (
        "guardrails",
        "failure handling",
        "guardrails / failure handling",
    ),
}
GENERIC_PHRASES = (
    "follow best practices",
    "ensure quality",
    "handle errors appropriately",
    "review the relevant files",
    "perform the task",
    "use this skill to use this skill",
    "this skill handles this skill",
    "as needed",
    # Paraphrases of the same emptiness. The blacklist is only the secondary
    # net: operational content is enforced structurally per procedure step
    # (action verb + concrete anchor) so a fresh paraphrase does not slip past.
    "adhere to best practices",
    "adhere to industry best practice",
    "apply best practices",
    "apply industry best practice",
    "as you see fit",
    "at your discretion",
    "follow industry best practice",
    "form a considered view",
    "form an opinion",
    "take a considered view",
    "use best practices",
    "use your best judgement",
    "use your best judgment",
    "use your own judgement",
    "use your own judgment",
)
STOPWORDS = {
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in",
    "into", "is", "it", "of", "on", "or", "that", "the", "their", "this",
    "to", "when", "with", "without", "must", "should", "will",
}
TARGET_DERIVED_KINDS = {
    "adapted", "project-adapted", "project_adapted", "project-derived",
    "project_derived", "integration", "specialty", "domain", "domain-review",
}
INTEGRATION_CATEGORIES = {"integration", "integrations"}
DOMAIN_CATEGORIES = {"domain", "domain-review", "domain_review", "domain review"}
SPECIALIST_CATEGORIES = (
    INTEGRATION_CATEGORIES
    | DOMAIN_CATEGORIES
    | {"specialty", "framework-specialty", "framework_specialty"}
)
IDENTITY_STOPWORDS = {
    "adapter", "contract", "domain", "integration", "review", "service", "services",
    "skill", "workflow",
}
LINE_FAIL_THRESHOLD = 0.70
TOKEN_FAIL_THRESHOLD = 0.80
TOKEN_WARN_THRESHOLD = 0.65
# Skeleton comparison (`_skeleton_line`/`SKILL_TEMPLATE_REUSE`). The raw
# thresholds above are calibrated on unmasked prose and mean something else
# once project identity is erased, so the skeleton pass carries its own
# numbers, measured rather than guessed. Honest corpus: 2371 pairs of
# hand-written accelerator skills (Symfony 1128, Laravel 990, this repo's own
# 253) plus the fixture's distinct pair - worst pair line=0.350 token=0.154,
# and that worst pair is a deliberately parallel scanner family; genuinely
# unrelated skills top out at line=0.231 token=0.107. Duplicate corpus: one
# template with the project's own nouns substituted line=0.400 token=0.329,
# the same template differing only in punctuation and connectives line=0.400
# token=0.311, a byte-identical clone line=0.500 token=0.471, the fixture's
# shared-template case line=1.000 token=1.000. The error line sits inside that
# gap on both axes, so a duplicate has to clear two independent metrics'
# margins to slip through, and the warning line sits above honest noise.
SKELETON_PLACEHOLDER = "xid"
_SKELETON_SUB = " " + SKELETON_PLACEHOLDER + " "
SKELETON_CONNECTIVES = {
    "additionally": "and",
    "afterwards": "then",
    "also": "and",
    "moreover": "and",
    "plus": "and",
    "subsequently": "then",
    "whilst": "while",
}
SKELETON_MIN_RESIDUAL = 3
SKELETON_MIN_LINES = 5
SKELETON_LINE_MATCH = 0.80
SKELETON_LINE_FAIL = 0.38
SKELETON_LINE_WARN = 0.28
SKELETON_TOKEN_FAIL = 0.26
SKELETON_TOKEN_WARN = 0.20
SKELETON_BLOCK_SIZE = 2
_CODE_SPAN_PATTERN = re.compile(r"`[^`]*`")
_PHP_VARIABLE_PATTERN = re.compile(
    r"\$[A-Za-z_][A-Za-z0-9_]*(?:\s*->\s*[A-Za-z0-9_]+)*"
)
_CALL_PATTERN = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\(\s*\)")
_PATH_PATTERN = re.compile(r"[A-Za-z0-9_.@-]*[\\/][A-Za-z0-9_.@\\/-]+")
_UPPER_SNAKE_PATTERN = re.compile(r"\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+\b")
_CAMEL_CASE_PATTERN = re.compile(r"\b[A-Za-z]+[a-z0-9]*(?:[A-Z][a-z0-9]*)+\b")
_DOTTED_ID_PATTERN = re.compile(r"\b[A-Za-z0-9]+(?:[_.:][A-Za-z0-9]+)+\b")
_NUMBER_PATTERN = re.compile(r"\b\d[\d,.]*\b")
_NON_WORD_PATTERN = re.compile(r"[^a-z0-9]+")
SUPPORTED_PLAN_SCHEMAS = {"1.0", "1.1", "1.2"}
ROUTING_ROLES = {"primary", "defer", "fallback"}
OWNERSHIP_MODES = {"exclusive", "shared", "composed"}
CAPABILITY_MODES = {"read-only", "workspace-write", "external-side-effect"}
PATH_ACCESS_MODES = {"read", "write"}
PATH_CLASSIFICATIONS = {"required-existing", "generated-runtime", "creatable"}
VERIFICATION_MODES = {"command", "manual"}
MUTATION_CLASSES = {"none", "workspace-write", "destructive"}
NETWORK_CLASSES = {"none", "local", "external-provider"}
NETWORK_POLICIES = {
    "forbidden",
    "mock-only",
    "sandbox-with-approval",
    "approved-live",
}
INTEGRATION_ENVIRONMENTS = {"none", "local", "sandbox", "approved-live"}
READ_ONLY_MUTATION_PATTERN = re.compile(
    r"\b(add|apply|create|delete|edit|fix|implement|modify|remove|rename|"
    r"rewrite|update|write)\b",
    re.I,
)
GENERIC_VERIFICATION_PATTERN = re.compile(
    r"\b(configured|evidenced|relevant|appropriate|narrow|target)\s+"
    r"(check|checks|command|commands|test|tests)\b|"
    r"\b(check|ensure|verify)\s+(it|quality|the result|everything)\b",
    re.I,
)
OWNERSHIP_ID_PATTERN = re.compile(
    r"^[a-z0-9]+(?:[.-][a-z0-9]+)*$"
)
CONTRACT_PROJECTION_FIELDS = (
    "necessity_rationale",
    "triggers",
    "owned_scope",
    "excluded_scope",
    "required_procedure_roles",
    "decision_points",
    "verification",
    "output_contract",
    "failure_handling",
)
# Skill prose is executed by the agent, so it is scanned for commands with the
# same analyzer the plan's verification commands use. Fenced shell blocks are
# unambiguous instructions and are read strictly; inline single-backtick spans
# also carry class names, config paths, constants, and code fragments, so they
# are gated to segments that actually look like an invocation.
SHELL_FENCE_LANGUAGES = frozenset({"bash", "console", "sh", "shell"})
# Executables whose bare form is also an ordinary English word or a PHP
# function name. A bare occurrence inside an inline span is prose; a
# path-qualified form (`bin/console`, `vendor/bin/...`) or a fenced shell
# block is not exempt.
AMBIGUOUS_BARE_EXECUTABLES = frozenset(
    {"artisan", "cap", "console", "deploy", "install", "test"}
)
COMMAND_RISK_CATEGORIES = frozenset(
    {
        "destructive_database_deploy",
        "external_provider_network",
        "workspace_mutation",
    }
)
# Verification blockers are about attestability, not danger, so prose is not
# failed for an unresolvable alias or an unknown executable. Privilege
# escalation is danger regardless of attestability.
COMMAND_BLOCKING_FINDINGS = frozenset({"SUDO_EXECUTION"})
FENCE_LINE_PATTERN = re.compile(r"^(?P<marker>`{3,}|~{3,})[ \t]*(?P<info>[^\s`]*)")
SHELL_PROMPT_PATTERN = re.compile(r"^[$>][ \t]+")
INLINE_CODE_PATTERN = re.compile(r"(?<!`)`([^`\n]+)`(?!`)")
ENV_ASSIGNMENT_TOKEN = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
UNICODE_SPACE_PATTERN = re.compile(
    "[\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]"
)
ZERO_WIDTH_PATTERN = re.compile("[\u200b-\u200d\u2060\ufeff]")
COMMAND_SEGMENT_SEPARATORS = frozenset(";|&<>()`\n\r")
# Safety cap on nested interpreter payloads analyzed per prose command.
MAX_BODY_COMMAND_SEGMENTS = 256
# A guardrail names the command it forbids, so prose polarity decides whether a
# backticked span is a USE (the skill tells the agent to run it) or a MENTION
# (the skill tells the agent never to). Without this, the safest possible
# guardrail - "Never run `rm -rf var/`" - is the one sentence that fails the
# gate. The markers are matched only inside the clause that carries the span,
# never across a whole section, so one prohibition cannot silence the scan.
# Fenced shell blocks are exempt from this relaxation by design: a ```bash
# block is an instruction to run, whatever the surrounding prose claims.
PROHIBITION_MARKERS = (
    "never",
    "do not",
    "don't",
    "does not",
    "must not",
    "cannot",
    "can't",
    "avoid",
    "refuse",
    "refrain",
    "forbidden",
    "prohibited",
    "not allowed",
    "no need to",
    "instead of",
    "rather than",
    "tempted to",
    "without running",
)
# Clause boundaries for the polarity window. Commas are deliberately absent:
# "Never run `x`, then `y`" must keep both spans inside the prohibition.
CLAUSE_BOUNDARY_PATTERN = re.compile(r"[.!?;\n\r]|—|--")

# Operational-content gate. A skill can carry real project nouns in every
# sentence and still tell the agent to do nothing, which passes every lexical
# and traceability check. Procedure steps are therefore read structurally: a
# step must command an action (a verb from the open technical set below, or a
# prescribed tool invocation) and the procedure must name something concrete.
# The blacklists are secondary; the verb/anchor structure is the mechanism.
#
# The set is open by design and is widened whenever honest instruction is
# measured to trip it, never narrowed to catch a specific miss: a gate that
# fails an honest generation is worse than the miss it closes. Two rules keep
# the widening principled instead of ad hoc - a verb enters when a measured
# honest step used it, and a verb whose counterpart is already listed
# ("upgrade"/"downgrade", "compute"/"recompute", "subscribe"/"unsubscribe",
# "serialize"/"deserialize", "modify"/"change", "bind"/"bound",
# "reject"/"accept", "stop"/"start") enters with it, because the omission was
# an oversight rather than a judgement. The counterpart rule stops at a word
# that is ordinarily an adjective or a noun: "close" stays out because
# "the closed invoice" must keep reading as a bare noun list.
ACTION_VERBS = frozenset(
    {
        "abort", "accept", "acknowledge", "add", "allow", "analyse", "analyze",
        "answer", "append",
        "apply", "approve", "ask", "assert", "assign", "attach", "audit",
        "author", "await", "benchmark",
        "bind", "block", "bound", "build", "cache", "calculate", "call",
        "cancel", "cap",
        "capture", "catalog", "catalogue", "categorise", "categorize", "catch",
        "change", "check", "choose", "cite", "clamp", "classify", "clone",
        "collect", "compare", "compile", "compute", "confirm", "constrain",
        "consume", "convert", "copy", "count", "cover", "create", "decide",
        "declare", "decode", "defer", "define", "delegate", "delete",
        "demand", "deny", "dequeue", "deploy", "derive", "describe",
        "deserialise", "deserialize", "detach",
        "detect", "diff", "disable",
        "disallow", "dispatch", "document", "downgrade", "draft", "draw",
        "drop", "dump",
        "edit", "elicit", "emit", "enable", "encode", "enforce", "enqueue",
        "ensure", "enumerate", "escalate", "estimate", "examine", "exclude",
        "execute", "exercise",
        "expand", "explain", "export", "extract", "fake", "fetch", "filter",
        "find", "fix", "flag", "flush", "follow", "forbid", "format",
        "forward", "frame", "gather", "generate", "give", "grep", "group",
        "guard", "halt", "hand", "handle", "highlight", "hold", "hydrate",
        "identify", "ignore",
        "implement", "import", "include", "index", "initialise", "initialize",
        "inject", "insert", "inspect", "install", "instantiate", "instrument",
        "invoke", "isolate", "iterate", "keep", "launch", "leave", "limit",
        "lint", "list", "load", "locate", "lock", "log", "maintain", "make",
        "map", "mark", "mask", "match", "measure", "merge", "migrate", "mock",
        "modify", "monitor", "move", "name", "normalise", "normalize", "note",
        "notify", "observe", "open", "outline", "paginate", "parse",
        "partition", "pass",
        "patch", "permit", "persist", "pick", "pin", "poll", "prefer",
        "prepend", "present", "preserve", "prevent", "print", "prioritise",
        "prioritize", "produce", "profile", "prohibit", "propose", "protect",
        "prove", "provide", "publish",
        "purge", "push", "query", "queue", "quote", "raise", "rank", "read",
        "rebuild", "recommend", "recompute", "record", "redact", "redeliver",
        "refactor",
        "refresh", "register", "reindex", "reject", "release", "reload",
        "remove",
        "rename", "render", "repair", "repeat", "replace", "replay", "report",
        "requeue", "require", "resend", "reset", "resolve", "restore",
        "restrict", "retry", "return", "reuse", "revert", "review", "rewrite",
        "rollback", "route", "run", "sanitise", "sanitize", "scan", "schedule",
        "search", "seed", "select", "send", "separate", "serialise",
        "serialize", "set", "show", "simulate", "skip", "sort", "specify",
        "split", "start", "state",
        "stop", "store", "stub", "subscribe", "suggest", "summarise",
        "summarize",
        "surface", "sync", "tag", "test", "throttle", "throw", "trace",
        "track", "transform", "translate", "traverse", "trigger", "truncate",
        "unlock", "unregister", "unset", "unsubscribe", "update", "upgrade",
        "upsert", "use",
        "validate", "verify", "wait", "walk", "warn", "watch", "wire", "wrap",
        "write",
    }
)
# A verification section exists to state a check, so it is read against the
# narrower set of verbs that describe running, observing, or asserting an
# outcome. A section naming only subject nouns ("Tenant scoping before offset
# and limit: fine.") states nothing an agent can execute or falsify.
VERIFICATION_ACTION_VERBS = frozenset(
    {
        "assert", "benchmark", "block", "catch", "check", "compare", "confirm",
        "contain", "count", "cover", "demonstrate", "detect", "diff", "emit",
        "ensure", "evaluate", "examine", "execute", "exercise", "exit",
        "expect", "fail", "grep", "hold", "inspect", "lint", "list", "match",
        "measure", "observe", "output", "pass", "print", "produce", "prove",
        "raise", "reject", "replay", "report", "reproduce", "resolve",
        "return", "review", "run", "scan", "show", "surface", "test", "throw",
        "trace", "validate", "verify", "watch", "yield",
    }
)
# A hedge is blocked where it governs the step: at the start of the step or of
# a sentence inside it. Mid-sentence hedging next to a real action verb stays
# legitimate, so an honest "Reject the allocation, keeping in mind ..." is not
# failed.
HEDGE_OPENERS = (
    "be aware", "be conscious", "be mindful", "bear in mind", "bearing in mind",
    "consider", "contemplate", "form a considered view", "form a view",
    "form an opinion", "have a look", "keep in mind", "keeping in mind",
    "mull over", "ponder", "reflect on", "reflect upon",
    "take a considered view", "take a holistic view", "take account of",
    "take into account", "taking into account", "think about", "think over",
    "think through",
)
# Hedges that are vacuous wherever they appear.
HEDGE_PHRASES = (
    "as you see fit", "at your discretion", "ensure everything",
    "ensure that everything", "form a considered view", "form an opinion",
    "make sure everything", "take a considered view",
    "use your best judgement", "use your best judgment",
)
HEDGE_OPENER_PATTERN = re.compile(
    r"^(?:"
    + "|".join(
        re.escape(phrase)
        for phrase in sorted(HEDGE_OPENERS, key=len, reverse=True)
    )
    + r")\b"
)
STEP_MARKER_PATTERN = re.compile(r"^(?:\d{1,3}[.)]|[-*+•]|[a-z][.)])\s+")
SENTENCE_SPLIT_PATTERN = re.compile(r"(?<=[.!?;:])\s+")
LEADING_CONNECTIVE_PATTERN = re.compile(
    r"^(?:additionally|afterwards|also|and|but|finally|first|firstly|"
    r"furthermore|instead|moreover|next|now|please|second|secondly|so|then|"
    r"therefore|third|thirdly)\b[\s,]*"
)
SUBORDINATE_CLAUSE_PATTERN = re.compile(
    r"^(?:after|before|if|once|unless|until|when|whenever|where|while)\b"
    r"[^,]{0,160},\s*"
)
WORD_PATTERN = re.compile(r"[a-z]+")
# A concrete anchor: something a reader can open, run, or grep for.
ANCHOR_PATTERNS = (
    re.compile(r"`[^`\n]+`"),
    re.compile(r"'[^'\n]{2,}'|\"[^\"\n]{2,}\""),
    re.compile(r"[\w.-]+/[\w.*/-]+"),
    re.compile(
        r"\w+\.(?:php|json|ya?ml|xml|md|twig|env|lock|ini|sql|neon|dist|toml|"
        r"js|ts|sh|py|txt|csv)\b",
        re.I,
    ),
    re.compile(r"::|->|\$\w+|\w+\(\)"),
    re.compile(r"\b[A-Za-z][a-z0-9]+[A-Z]\w*"),
    re.compile(r"\b[A-Z][A-Z0-9]{3,}\b|\b[A-Z][A-Z0-9]*_[A-Z0-9_]+\b"),
    re.compile(r"\b[a-z0-9]+_[a-z0-9_]+\b"),
    re.compile(r"\b[a-z0-9]+(?:-[a-z0-9]+)+\b"),
    re.compile(r"\b\d+\b"),
)
NON_FALSIFIABLE_VERIFICATION_PATTERN = re.compile(
    r"\b(?:look|looks|looked|looking|seem|seems|seemed|appear|appears|"
    r"appeared|feel|feels)\s+(?:to\s+be\s+)?(?:acceptable|correct|fine|good|"
    r"healthy|ok|okay|plausible|reasonable|right|sane|sensible|sound)\b"
    r"|\bnothing\s+(?:appears|is|looks|seems)\s+(?:amiss|broken|off|"
    r"suspicious|unusual|wrong)\b"
    r"|\beverything\s+(?:appears|is|looks|seems)\s+(?:correct|fine|good|"
    r"in order|ok|okay|right|as expected)\b",
    re.I,
)
MAX_DIAGNOSTIC_EXCERPT = 72


@dataclass(frozen=True, order=True)
class Diagnostic:
    severity: str
    code: str
    message: str

    def as_dict(self) -> dict[str, str]:
        return {"severity": self.severity, "code": self.code, "message": self.message}


def _diag(items: list[Diagnostic], code: str, message: str, severity: str = "error") -> None:
    items.append(Diagnostic(severity, code, message))


def _is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _is_string_list(value: Any, *, allow_empty: bool = False) -> bool:
    return (
        isinstance(value, list)
        and (allow_empty or bool(value))
        and all(_is_nonempty_string(item) for item in value)
    )


def _flatten_strings(value: Any) -> list[str]:
    if isinstance(value, str):
        return [value]
    if isinstance(value, list):
        flattened: list[str] = []
        for item in value:
            flattened.extend(_flatten_strings(item))
        return flattened
    if isinstance(value, dict):
        flattened = []
        for key in sorted(value):
            flattened.extend(_flatten_strings(value[key]))
        return flattened
    return []


def _as_list(value: Any) -> list[Any]:
    """Project a possibly wrong-typed plan field to a safe iterable."""
    return value if isinstance(value, list) else []


def _is_substantive_contract(value: Any, *, allow_empty: bool = False) -> bool:
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, list):
        if not value:
            return allow_empty
        return all(
            isinstance(item, (str, dict))
            and bool(_flatten_strings(item))
            and all(part.strip() for part in _flatten_strings(item))
            for item in value
        )
    return False


def _fixed_block_contents(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [
        item["content"]
        for item in value
        if isinstance(item, dict)
        and _is_nonempty_string(item.get("id"))
        and _is_nonempty_string(item.get("version"))
        and _is_nonempty_string(item.get("content"))
    ]


def _safe_relative(value: Any) -> bool:
    if not _is_nonempty_string(value):
        return False
    path = Path(value)
    return not path.is_absolute() and ".." not in path.parts and value not in (".", "./")


def _confined(root: Path, relative: str) -> Path | None:
    if not _safe_relative(relative):
        return None
    candidate = root
    for part in Path(relative).parts:
        candidate = candidate / part
        if candidate.is_symlink():
            return None
    try:
        candidate.resolve().relative_to(root.resolve())
    except (OSError, ValueError):
        return None
    return candidate


def _approved_url(value: str) -> bool:
    parsed = urlparse(value)
    return (
        parsed.scheme in {"http", "https"}
        and bool(parsed.hostname)
        and parsed.username is None
        and parsed.password is None
        and not parsed.fragment
    )


def _evidence_location(entry: dict[str, Any]) -> tuple[str, str] | None:
    candidates = [
        ("path", entry.get("path")),
        ("url", entry.get("url")),
        ("source", entry.get("source")),
    ]
    populated = [(key, value.strip()) for key, value in candidates if _is_nonempty_string(value)]
    if len(populated) != 1:
        return None
    key, value = populated[0]
    if key == "source":
        key = "url" if value.startswith(("http://", "https://")) else "path"
    return key, value


def _load_plan(path: Path, diagnostics: list[Diagnostic]) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        _diag(diagnostics, "PLAN_UNREADABLE", f"plan is not readable JSON: {error}")
        return {}
    if not isinstance(value, dict):
        _diag(diagnostics, "PLAN_TYPE", "plan root must be an object")
        return {}
    return value


def _heading_slug(value: str) -> str:
    value = re.sub(r"[`*_]", "", value.strip().lower())
    value = re.sub(r"[^a-z0-9 -]", "", value)
    return re.sub(r"[- ]+", "-", value).strip("-")


def _load_registry(
    path: Path, catalog_version: Any, diagnostics: list[Diagnostic]
) -> dict[str, dict[str, str]]:
    try:
        registry = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        _diag(
            diagnostics,
            "REGISTRY_INVALID",
            f"candidate registry unreadable or invalid: {error}",
        )
        return {}
    required_registry_fields = {"schema_version", "catalog_version", "candidates"}
    optional_registry_fields = {"runtime_contract", "compilation_requirements"}
    if (
        not isinstance(registry, dict)
        or not required_registry_fields <= set(registry)
        or not set(registry) <= required_registry_fields | optional_registry_fields
    ):
        _diag(diagnostics, "REGISTRY_INVALID", "candidate registry fields are invalid")
        return {}
    if (
        registry.get("schema_version") != "1.0"
        or registry.get("catalog_version") != catalog_version
        or not isinstance(registry.get("candidates"), list)
    ):
        _diag(
            diagnostics,
            "REGISTRY_VERSION",
            "candidate registry schema/catalog version does not match plan",
        )
        return {}
    candidates: dict[str, dict[str, str]] = {}
    for index, item in enumerate(registry["candidates"]):
        if (
            not isinstance(item, dict)
            or set(item) != {"id", "catalog", "category", "mode"}
            or not all(_is_nonempty_string(item.get(key)) for key in item)
            or item.get("mode") not in {"static", "runtime-fixed", "family"}
        ):
            _diag(
                diagnostics,
                "REGISTRY_CANDIDATE_INVALID",
                f"candidate registry entry {index} is invalid",
            )
            continue
        candidate_id = item["id"]
        if candidate_id in candidates:
            _diag(
                diagnostics,
                "REGISTRY_CANDIDATE_DUPLICATE",
                f"duplicate candidate registry id: {candidate_id}",
            )
            continue
        catalog_parts = item["catalog"].split("#", 1)
        catalog_file = path.parent / catalog_parts[0]
        if (
            len(catalog_parts) != 2
            or not catalog_file.is_file()
            or not catalog_parts[1]
        ):
            _diag(
                diagnostics,
                "REGISTRY_CATALOG_INVALID",
                f"{candidate_id}: catalog reference does not resolve",
            )
        else:
            headings = {
                _heading_slug(match.group(1))
                for match in re.finditer(
                    r"^#{1,6}\s+(.+)$",
                    catalog_file.read_text(encoding="utf-8"),
                    re.M,
                )
            }
            if catalog_parts[1] not in headings:
                _diag(
                    diagnostics,
                    "REGISTRY_CATALOG_ANCHOR",
                    f"{candidate_id}: catalog anchor does not resolve: {item['catalog']}",
                )
        candidates[candidate_id] = item
    return candidates


def _parse_frontmatter(text: str) -> tuple[dict[str, str], str]:
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return {}, text
    try:
        end = next(index for index in range(1, len(lines)) if lines[index].strip() == "---")
    except StopIteration:
        return {}, text
    fields: dict[str, str] = {}
    index = 1
    while index < end:
        match = re.match(r"^([A-Za-z][A-Za-z0-9_-]*):\s*(.*?)\s*$", lines[index])
        index += 1
        if not match:
            continue
        value = match.group(2).strip().strip("\"'")
        if re.fullmatch(r"[>|][+-]?", value):
            continuation: list[str] = []
            while index < end and (
                not lines[index].strip() or lines[index][:1].isspace()
            ):
                continuation.append(lines[index].strip())
                index += 1
            value = " ".join(part for part in continuation if part)
        fields[match.group(1)] = value
    return fields, "\n".join(lines[end + 1 :])


def _sections(body: str) -> dict[str, str]:
    found: dict[str, list[str]] = {}
    current = ""
    for line in body.splitlines():
        heading = re.match(r"^#{2,6}\s+(.+?)\s*$", line)
        if heading:
            current = re.sub(r"[*_`]", "", heading.group(1)).strip().lower()
            found.setdefault(current, [])
        elif current:
            found[current].append(line)
    return {name: "\n".join(lines).strip() for name, lines in found.items()}


def _section(sections: dict[str, str], aliases: Iterable[str]) -> str:
    for alias in aliases:
        if alias in sections:
            return sections[alias]
    return ""


def _tokens(value: str) -> list[str]:
    tokens: list[str] = []
    for raw in re.findall(r"[a-z0-9][a-z0-9_.:/-]*", value.lower()):
        token = raw.rstrip(".:/")
        if token and token not in STOPWORDS and len(token) > 1:
            tokens.append(token)
    return tokens


def _meaningful_tokens(value: str) -> set[str]:
    return {token for token in _tokens(value) if len(token) >= 3 or any(char.isdigit() for char in token)}


def _name_pattern(name: str) -> str:
    """Regex-safe pattern matching a skill name with '-' or ' ' separators."""
    return r"[- ]".join(re.escape(part) for part in name.split("-"))


def _contract_matches(contract: Any, text: str) -> bool:
    values = _flatten_strings(contract)
    expected = set().union(*(_meaningful_tokens(item) for item in values))
    if not expected:
        return False
    actual = _meaningful_tokens(text)
    required = 1 if len(expected) < 4 else 2
    return len(expected & actual) >= required


def _verb_forms(verb: str) -> set[str]:
    """Expand one base verb to the surface forms procedure prose uses."""
    forms = {verb, verb + "s", verb + "ing", verb + "ed"}
    if verb.endswith("e"):
        forms.add(verb[:-1] + "ing")
        forms.add(verb + "d")
    if verb.endswith(("s", "x", "z", "ch", "sh")):
        forms.add(verb + "es")
    if verb.endswith("y") and len(verb) > 2 and verb[-2] not in "aeiou":
        forms.add(verb[:-1] + "ies")
        forms.add(verb[:-1] + "ied")
    return forms


ACTION_VERB_FORMS = frozenset(
    form for verb in ACTION_VERBS for form in _verb_forms(verb)
)
VERIFICATION_VERB_FORMS = frozenset(
    form for verb in VERIFICATION_ACTION_VERBS for form in _verb_forms(verb)
)


def _excerpt(value: str) -> str:
    """Quote a bounded, whitespace-collapsed excerpt for a diagnostic."""
    collapsed = " ".join(value.split())
    if len(collapsed) > MAX_DIAGNOSTIC_EXCERPT:
        collapsed = collapsed[: MAX_DIAGNOSTIC_EXCERPT - 3] + "..."
    return repr(collapsed)


def _procedure_steps(section: str) -> list[str]:
    """Split a procedure section into the steps an agent would follow.

    Flush-left ordered or bulleted markers delimit steps; wrapped lines,
    indented sub-bullets, and fenced blocks continue the step they belong to,
    and any lead-in before the first marker is not a step. A section written
    as prose falls back to blank-line paragraphs, so an unlisted procedure is
    read as one instruction rather than as many fragments.
    """
    listed: list[list[str]] = []
    paragraphs: list[list[str]] = []
    current: list[str] | None = None
    fence = ""
    for raw in section.splitlines():
        stripped = raw.strip()
        if fence:
            if (
                stripped
                and len(stripped) >= len(fence)
                and set(stripped) == {fence[0]}
            ):
                fence = ""
            elif current is not None:
                current.append(stripped)
            continue
        opening = FENCE_LINE_PATTERN.match(stripped)
        if opening:
            fence = opening.group("marker")
            continue
        if STEP_MARKER_PATTERN.match(raw):
            current = [STEP_MARKER_PATTERN.sub("", raw, count=1).strip()]
            listed.append(current)
            continue
        if not stripped:
            if not listed:
                current = None
            continue
        if current is None:
            current = []
            paragraphs.append(current)
        current.append(stripped)
    buckets = listed or paragraphs
    return [
        text for text in (" ".join(bucket).strip() for bucket in buckets) if text
    ]


def _has_action_verb(text: str, vocabulary: frozenset = ACTION_VERB_FORMS) -> bool:
    """True when the text names an action from the given verb vocabulary."""
    return any(
        word in vocabulary
        for word in WORD_PATTERN.findall(_normalize_command_text(text).lower())
    )


def _prescribes_tooling(text: str) -> bool:
    """True when the text hands the agent an invocation instead of a phrase.

    A bare command is written lowercase, so a sentence that merely opens with
    a capitalized word which happens to name a CLI ("Firebase initialization
    and the messaging boundary: fine.") is prose, not an invocation.
    """
    for segment in _command_segments(_normalize_command_text(text)):
        head = segment.split()[0] if segment.split() else ""
        if head[:1].isupper():
            continue
        if _prescribes_command(segment, strict=False):
            return True
    return False


def _commands_action(text: str) -> bool:
    return _has_action_verb(text) or _prescribes_tooling(text)


def _clause_openings(sentence: str) -> list[str]:
    """Return the governing openings of a sentence.

    A leading connective ("Then, ...") and one leading subordinate clause
    ("When the guard trips, ...") do not carry the instruction, so the text
    behind them is tested as an opening too.
    """
    opening = LEADING_CONNECTIVE_PATTERN.sub("", sentence.strip(" \t\"'`*_-([")).strip()
    openings = [opening]
    remainder = SUBORDINATE_CLAUSE_PATTERN.sub("", opening).strip()
    if remainder and remainder != opening:
        openings.append(LEADING_CONNECTIVE_PATTERN.sub("", remainder).strip())
    return openings


def _hedge_phrase(text: str) -> str | None:
    """Return the hedge that governs the text, or None.

    Deterministic: the anywhere-blacklist is scanned in its declared order,
    then sentence openings left to right, longest hedge first.
    """
    lowered = _normalize_command_text(text).lower()
    for phrase in HEDGE_PHRASES:
        if phrase in lowered:
            return phrase
    for sentence in SENTENCE_SPLIT_PATTERN.split(lowered):
        for opening in _clause_openings(sentence):
            match = HEDGE_OPENER_PATTERN.match(opening)
            if match:
                return match.group(0)
    return None


def _has_concrete_anchor(text: str) -> bool:
    """True when the text names a path, symbol, constant, command, or number."""
    return any(pattern.search(text) for pattern in ANCHOR_PATTERNS)


def _normalize_command_text(value: str) -> str:
    """Fold invisible separators so a homoglyph cannot hide an executable.

    ``rm -rf`` tokenizes as one unknown word; normalizing the non-breaking
    space (and dropping zero-width joiners) makes it the ``rm`` it will be when
    an agent retypes the line into a shell.
    """
    value = ZERO_WIDTH_PATTERN.sub("", value)
    return UNICODE_SPACE_PATTERN.sub(" ", value)


def _shell_code_blocks(body: str) -> list[list[str]]:
    """Return the raw lines of every fenced bash/sh/shell/console block."""
    blocks: list[list[str]] = []
    marker = ""
    language = ""
    collected: list[str] = []
    for line in body.splitlines():
        stripped = line.strip()
        if not marker:
            opening = FENCE_LINE_PATTERN.match(stripped)
            if opening:
                marker = opening.group("marker")
                language = opening.group("info").strip().lower()
                collected = []
            continue
        if (
            stripped
            and len(stripped) >= len(marker)
            and set(stripped) == {marker[0]}
        ):
            if language in SHELL_FENCE_LANGUAGES:
                blocks.append(collected)
            marker = ""
            language = ""
            collected = []
            continue
        collected.append(line)
    if marker and language in SHELL_FENCE_LANGUAGES:
        blocks.append(collected)
    return blocks


def _shell_block_commands(lines: list[str]) -> list[str]:
    """Project fenced-block lines to command lines.

    Backslash continuations are joined, shell comments are dropped, and a
    leading ``$``/``>`` transcript prompt is removed so a pasted session still
    exposes its commands.
    """
    joined: list[str] = []
    pending = ""
    for raw in lines:
        line = _normalize_command_text(raw).strip()
        if pending:
            line = f"{pending} {line}".strip()
            pending = ""
        if line.endswith("\\"):
            pending = line[:-1].strip()
            continue
        joined.append(line)
    if pending:
        joined.append(pending)
    commands: list[str] = []
    for line in joined:
        line = SHELL_PROMPT_PATTERN.sub("", line).strip()
        if not line or line.startswith("#"):
            continue
        commands.append(line)
    return commands


def _forbids_span(body: str, start: int) -> bool:
    """Report whether the clause carrying ``start`` forbids its command.

    The window is the text between the last clause boundary before the span
    and the span itself, so a prohibition governs only its own clause: in
    ``Do not run `a` - run `b` instead`` the first span is a mention and the
    second stays a prescription.
    """
    window = body[:start]
    boundaries = list(CLAUSE_BOUNDARY_PATTERN.finditer(window))
    if boundaries:
        window = window[boundaries[-1].end():]
    lowered = window.lower()
    return any(marker in lowered for marker in PROHIBITION_MARKERS)


def _body_commands(body: str) -> list[tuple[str, bool]]:
    """Return ``(command, strict)`` pairs an agent would execute from prose."""
    found: list[tuple[str, bool]] = []
    for block in _shell_code_blocks(body):
        for command in _shell_block_commands(block):
            found.append((command, True))
    for match in INLINE_CODE_PATTERN.finditer(body):
        span = _normalize_command_text(match.group(1)).strip()
        # A span the prose forbids is quoted, not prescribed: skipping it keeps
        # the safest guardrail a skill can carry from failing the gate.
        if span and not _forbids_span(body, match.start()):
            found.append((span, False))
    return found


def _command_segments(command: str) -> list[str]:
    """Split a command at quote-aware shell separators.

    The analyzer refuses to classify a composed command, so ``rm -rf x && curl
    -X POST url`` would otherwise report only that it is composed. Splitting
    first means every leaf is classified on its own.
    """
    segments: list[str] = []
    current: list[str] = []
    quote = ""
    for char in command:
        if quote:
            current.append(char)
            if char == quote:
                quote = ""
            continue
        if char in "'\"":
            quote = char
            current.append(char)
            continue
        if char in COMMAND_SEGMENT_SEPARATORS:
            segments.append("".join(current).strip())
            current = []
            continue
        current.append(char)
    segments.append("".join(current).strip())
    return [segment for segment in segments if segment]


def _prescribes_command(segment: str, *, strict: bool) -> bool:
    """Decide whether a prose segment is an invocation rather than a phrase.

    A fenced shell block is strict: any segment naming a known executable is a
    command. An inline span must additionally carry an argument, and a bare
    English-word executable (``test``, ``install``) only counts when it is
    path-qualified, so class names, config paths, constants, YAML keys, and PHP
    fragments in backticks are never mistaken for commands.
    """
    tokens = segment.split()
    index = 0
    while index < len(tokens) and ENV_ASSIGNMENT_TOKEN.match(tokens[index]):
        index += 1
    if index >= len(tokens):
        return False
    head = tokens[index]
    executable = Path(head).name.lower()
    if executable not in RUNNER_EXECUTABLES:
        return False
    if strict:
        return True
    if len(tokens) - index < 2:
        return False
    return not (executable in AMBIGUOUS_BARE_EXECUTABLES and "/" not in head)


def _hosted_code_arguments(segment: str) -> list[str]:
    """Return the arguments an interpreter would execute as code.

    ``bash -c "curl -X POST https://host/x"`` keeps its payload inside one
    quoted token, so segment splitting alone would only see the shell. The
    payload of a code-hosting executable is re-read as a nested command; any
    other executable's arguments are left alone, so ``grep -rn "rm -rf" src/``
    stays a grep.
    """
    tokens = segment.split()
    if not tokens or Path(tokens[0]).name.lower() not in CODE_HOST_EXECUTABLES:
        return []
    try:
        parsed = shlex.split(segment, posix=True)
    except ValueError:
        return []
    return [
        token
        for token in parsed[1:]
        if token.strip() and not token.startswith("-")
    ]


def _command_risk_codes(analysis: Any) -> list[str]:
    """Return the danger findings of an analysis, ignoring attestability."""
    return sorted(
        {
            finding.code
            for finding in analysis.findings
            if finding.category in COMMAND_RISK_CATEGORIES
            or finding.code in COMMAND_BLOCKING_FINDINGS
        }
    )


def _validate_body_commands(
    name: str,
    body: str,
    target: Path,
    diagnostics: list[Diagnostic],
) -> None:
    """Analyze the commands a skill body hands to the agent."""
    commands = _body_commands(body)
    if not commands:
        return
    try:
        analyzer = CommandAnalyzer(target)
    except CommandAnalysisError as error:
        _diag(
            diagnostics,
            "COMMAND_ANALYZER_UNAVAILABLE",
            f"{name} target commands cannot be analyzed safely: {error}",
        )
        return
    for command, strict in commands:
        # Breadth-first over the written order. A LIFO walk pops the tail
        # first, so a chain longer than the cap would drop its head - exactly
        # where a dangerous command hides behind harmless padding.
        pending = deque(
            (segment, strict) for segment in _command_segments(command)
        )
        seen: set[tuple[str, bool]] = set()
        while pending:
            if len(seen) >= MAX_BODY_COMMAND_SEGMENTS:
                # Never drop the remainder silently: an unscanned tail is an
                # unproven command, and unproven fails closed.
                _diag(
                    diagnostics,
                    "SKILL_BODY_COMMAND_UNSCANNED",
                    f"{name} prose command exceeds the "
                    f"{MAX_BODY_COMMAND_SEGMENTS}-segment analysis cap and "
                    f"cannot be proven safe: {command}",
                )
                break
            entry = pending.popleft()
            if entry in seen:
                continue
            seen.add(entry)
            segment, segment_strict = entry
            if not _prescribes_command(segment, strict=segment_strict):
                continue
            for hosted in _hosted_code_arguments(segment):
                # Hosted code is unambiguously an instruction, never prose,
                # so the nested pass is always strict.
                pending.extend(
                    (nested, True) for nested in _command_segments(hosted)
                )
            try:
                analysis = analyzer.analyze(segment)
            except CommandAnalysisError:
                continue
            codes = _command_risk_codes(analysis)
            if codes:
                _diag(
                    diagnostics,
                    "SKILL_BODY_COMMAND_RISK",
                    f"{name}: skill body prescribes an unsafe command "
                    f"({', '.join(codes)}): {segment}",
                )


def _normalize_line(line: str) -> str:
    line = re.sub(r"TASK-\d+", "task-id", line, flags=re.IGNORECASE)
    line = re.sub(r"\bprofile[-_][a-z0-9_-]+\b", "profile-id", line, flags=re.IGNORECASE)
    line = re.sub(r"\s+", " ", line.strip().lower())
    return line


def _normalized_body(body: str, fixed_blocks: list[str]) -> tuple[list[str], list[str]]:
    cleaned = body
    for block in sorted(fixed_blocks, key=lambda item: (-len(item), item)):
        cleaned = cleaned.replace(block, "")
    lines: list[str] = []
    for raw in cleaned.splitlines():
        if re.match(r"^#{1,6}\s+", raw) or not raw.strip():
            continue
        normalized = _normalize_line(raw)
        if normalized:
            lines.append(normalized)
    return lines, _tokens("\n".join(lines))


def _line_similarity(left: list[str], right: list[str]) -> float:
    if not left or not right:
        return 0.0
    overlap = sum((Counter(left) & Counter(right)).values())
    return overlap / min(len(left), len(right))


def _ngrams(tokens: list[str], size: int = 3) -> set[tuple[str, ...]]:
    if len(tokens) < size:
        return {(token,) for token in tokens}
    return {tuple(tokens[index : index + size]) for index in range(len(tokens) - size + 1)}


def _token_similarity(left: list[str], right: list[str]) -> float:
    left_set, right_set = _ngrams(left), _ngrams(right)
    union = left_set | right_set
    return len(left_set & right_set) / len(union) if union else 0.0


def _skeleton_line(raw: str, entities: Iterable[str] = ()) -> str:
    """Erase project identity from a line, keeping only its prose skeleton.

    `_normalize_line` folds case and whitespace and nothing else, so two
    sentences produced from one template score as unrelated as soon as the
    generator substitutes the project's own nouns: a plan-conforming SKILL.md
    is *required* to carry its claim, its paths and its neighbours' names, and
    those mandatory differences dilute raw similarity below any usable
    threshold. Removing exactly that mandated vocabulary leaves the reusable
    scaffolding, which is what a duplicate actually is.

    Erased: backticked spans, paths, PHP variables/calls, CamelCase and
    UPPER_SNAKE identifiers, dotted/underscored ids, numbers, and the caller's
    entity names (the inventory's other skill names). Punctuation is dropped
    and a small set of interchangeable connectives is folded, so swapping
    `,` for `;` or `and` for `plus` no longer produces a different line.
    """
    text = _CODE_SPAN_PATTERN.sub(_SKELETON_SUB, raw)
    text = _PHP_VARIABLE_PATTERN.sub(_SKELETON_SUB, text)
    text = _CALL_PATTERN.sub(_SKELETON_SUB, text)
    text = _PATH_PATTERN.sub(_SKELETON_SUB, text)
    text = _UPPER_SNAKE_PATTERN.sub(_SKELETON_SUB, text)
    text = _CAMEL_CASE_PATTERN.sub(_SKELETON_SUB, text)
    text = _DOTTED_ID_PATTERN.sub(_SKELETON_SUB, text)
    text = _NUMBER_PATTERN.sub(_SKELETON_SUB, text)
    for entity in sorted(entities, key=lambda item: (-len(item), item)):
        if len(entity) >= 3:
            text = re.sub(
                r"\b" + re.escape(entity) + r"\b", _SKELETON_SUB, text, flags=re.I
            )
    text = _NON_WORD_PATTERN.sub(" ", text.lower())
    return " ".join(
        SKELETON_CONNECTIVES.get(word, word) for word in text.split()
    ).strip()


def _skeleton_body(
    body: str, fixed_blocks: list[str], entities: Iterable[str] = ()
) -> list[str]:
    """Skeleton lines that still carry enough prose of their own to compare.

    A line that is almost entirely project identity ("Read <path>.") collapses
    to placeholders; counting it would make every honest inventory look like a
    template, so only lines with at least `SKELETON_MIN_RESIDUAL` remaining
    content words take part in the comparison.

    Fenced blocks are dropped as well: a shared code idiom or an identical
    snippet of framework boilerplate is legitimate reuse, and on the skeleton
    two such fences collapse onto each other whatever the surrounding prose
    says. The check is about duplicated *instructions*, not duplicated code.
    """
    cleaned = body
    for block in sorted(fixed_blocks, key=lambda item: (-len(item), item)):
        cleaned = cleaned.replace(block, "")
    entities = tuple(entities)
    lines: list[str] = []
    fenced = False
    for raw in cleaned.splitlines():
        if re.match(r"^\s*(```|~~~)", raw):
            fenced = not fenced
            continue
        if fenced or re.match(r"^#{1,6}\s+", raw) or not raw.strip():
            continue
        line = _skeleton_line(raw, entities)
        residual = [
            word
            for word in line.split()
            if word != SKELETON_PLACEHOLDER
            and word not in STOPWORDS
            and len(word) > 1
        ]
        if len(residual) >= SKELETON_MIN_RESIDUAL:
            lines.append(line)
    return lines


def _bag_similarity(left: Counter, right: Counter) -> float:
    """Dice coefficient over word bags: `difflib.quick_ratio` without difflib."""
    total = sum(left.values()) + sum(right.values())
    if not total:
        return 0.0
    return 2 * sum((left & right).values()) / total


def _skeleton_line_similarity(left: list[str], right: list[str]) -> float:
    """Fraction of the shorter body whose lines have a twin in the other body.

    Two skeleton lines are twins when their word bags reach
    `SKELETON_LINE_MATCH`; that tolerance is what survives one substituted
    noun in an otherwise identical sentence, which exact comparison cannot.
    Matching is greedy over a deterministic order and one-to-one, so a single
    boilerplate line cannot claim several partners.
    """
    if not left or not right:
        return 0.0
    left_bags = [Counter(line.split()) for line in left]
    right_bags = [Counter(line.split()) for line in right]
    index: defaultdict[str, set[int]] = defaultdict(set)
    for position, bag in enumerate(right_bags):
        for word in bag:
            if word != SKELETON_PLACEHOLDER and word not in STOPWORDS and len(word) > 1:
                index[word].add(position)
    taken: set[int] = set()
    matched = 0
    for bag in left_bags:
        candidates: set[int] = set()
        for word in bag:
            candidates |= index.get(word, set())
        best_score, best_position = 0.0, None
        for position in sorted(candidates - taken):
            score = _bag_similarity(bag, right_bags[position])
            if score > best_score:
                best_score, best_position = score, position
        if best_position is not None and best_score >= SKELETON_LINE_MATCH:
            taken.add(best_position)
            matched += 1
    return matched / min(len(left), len(right))


def _compare_skeletons(
    left_name: str,
    left: list[str],
    right_name: str,
    right: list[str],
    diagnostics: list[Diagnostic],
) -> None:
    """Report two skills written from one template once identity is erased.

    Short bodies are skipped: below `SKELETON_MIN_LINES` comparable lines a
    single shared sentence already moves the ratio far enough to accuse an
    honest skill, and the raw `SKILL_SIMILARITY` pass still covers those.
    """
    if len(left) < SKELETON_MIN_LINES or len(right) < SKELETON_MIN_LINES:
        return
    line_score = _skeleton_line_similarity(left, right)
    token_score = _token_similarity(
        _tokens("\n".join(left)), _tokens("\n".join(right))
    )
    if line_score >= SKELETON_LINE_FAIL or token_score >= SKELETON_TOKEN_FAIL:
        shared = sorted(set(left) & set(right))[:2]
        detail = "; shared skeleton: " + " | ".join(shared) if shared else ""
        _diag(
            diagnostics,
            "SKILL_TEMPLATE_REUSE",
            f"{left_name} and {right_name} are the same prose template once "
            f"paths, identifiers, constants, numbers and neighbour names are "
            f"removed (line={line_score:.3f}, token={token_score:.3f})"
            f"{detail}",
        )
    elif line_score >= SKELETON_LINE_WARN or token_score >= SKELETON_TOKEN_WARN:
        _diag(
            diagnostics,
            "SKILL_TEMPLATE_REUSE_WARN",
            f"{left_name} and {right_name} share prose scaffolding once project "
            f"identity is removed (line={line_score:.3f}, token={token_score:.3f})",
            "warning",
        )


def _scope_parts(values: list[str]) -> set[str]:
    return {
        re.sub(r"/+$", "", value.strip().lower())
        for value in _flatten_strings(values)
    }


def _scopes_collide(left: set[str], right: set[str]) -> set[str]:
    collisions: set[str] = set()
    for one in left:
        for two in right:
            if one == two or one.startswith(two + "/") or two.startswith(one + "/"):
                collisions.add(one if len(one) <= len(two) else two)
    return collisions


def _normalize_glob(value: str) -> str:
    normalized = value.strip().replace("\\", "/")
    while normalized.startswith("./"):
        normalized = normalized[2:]
    normalized = re.sub(r"/+", "/", normalized).rstrip("/")
    return normalized


def _glob_prefix(value: str) -> str:
    wildcard = min(
        (index for token in "*?[" if (index := value.find(token)) >= 0),
        default=len(value),
    )
    return value[:wildcard].rstrip("/")


def _glob_suffix(value: str) -> str | None:
    """Literal tail after the last ``*``/``?`` wildcard (whole value if none).

    Returns ``None`` when the tail contains a ``[`` character class: the text a
    class matches is not part of the literal suffix, so the tail cannot be
    proven literal and callers must treat the suffix as unknown.
    """
    index = max(value.rfind("*"), value.rfind("?"))
    suffix = value if index < 0 else value[index + 1 :]
    return None if "[" in suffix else suffix


def _bare_directory_pattern(value: str) -> bool:
    """True for a wildcard-free pattern whose final segment has no extension.

    ``_normalize_glob`` accepts bare directory writes (``docs/sub/`` becomes
    ``docs/sub``), so such a pattern may denote a whole directory tree rather
    than a single file.  Its effective literal suffix is therefore unknown and
    must not participate in the suffix disjointness proof: ``docs/sub`` can
    contain ``docs/sub/notes.md``, which fnmatch's slash-crossing ``*`` lets
    ``docs/*.md`` match.  Wildcard-free names WITH an extension (e.g.
    ``docs/CHANGELOG.json``) are deliberately treated as single files.
    """
    if any(token in value for token in "*?["):
        return False
    return "." not in value.rsplit("/", 1)[-1]


def _globs_intersect(left: str, right: str) -> bool:
    """Conservatively determine whether two normalized target globs intersect."""
    left = _normalize_glob(left).lower()
    right = _normalize_glob(right).lower()
    if not left or not right:
        return False
    if fnmatch.fnmatchcase(left, right) or fnmatch.fnmatchcase(right, left):
        return True
    left_wild = any(token in left for token in "*?[")
    right_wild = any(token in right for token in "*?[")
    bare_containment = (
        (not left_wild and right.startswith(left + "/"))
        or (not right_wild and left.startswith(right + "/"))
    )
    left_suffix = None if _bare_directory_pattern(left) else _glob_suffix(left)
    right_suffix = None if _bare_directory_pattern(right) else _glob_suffix(right)
    if (
        not bare_containment
        and left_suffix is not None
        and right_suffix is not None
        and not left_suffix.endswith(right_suffix)
        and not right_suffix.endswith(left_suffix)
    ):
        # Every path matched by a glob ends with its literal suffix, so
        # incompatible suffixes (e.g. *.md vs *.json) are provably disjoint.
        # Suffixes containing a character class are unknown (None), as are
        # bare directory patterns (which may denote whole trees); both fall
        # through to the conservative prefix heuristic below.
        return False
    left_prefix, right_prefix = _glob_prefix(left), _glob_prefix(right)
    if not left_prefix or not right_prefix:
        return True
    return (
        left_prefix == right_prefix
        or left_prefix.startswith(right_prefix + "/")
        or right_prefix.startswith(left_prefix + "/")
    )


def _write_collisions(left: list[str], right: list[str]) -> set[str]:
    return {
        min(_normalize_glob(one), _normalize_glob(two), key=lambda item: (len(item), item))
        for one in left
        for two in right
        if _globs_intersect(one, two)
    }


def _writes_of(skill: dict[str, Any]) -> list[str]:
    """Declared write globs of a skill, tolerant of a wrong-typed field."""
    return [item for item in _as_list(skill.get("writes")) if isinstance(item, str)]


def _verbatim_boundary_fields(skill: dict[str, Any]) -> dict[str, set[str]]:
    """The boundary text a skill claims, normalized for comparison only.

    Case and whitespace are folded so a reflowed contract still counts as the
    same sentence; nothing else is relaxed, because the point of the check is
    literal identity.
    """
    descriptions = [
        item.get("description")
        for item in _as_list(skill.get("ownership"))
        if isinstance(item, dict)
    ]
    return {
        field: {
            normalized
            for raw in _flatten_strings(value)
            if (normalized := _normalize_line(raw))
        }
        for field, value in (
            ("owned_scope", skill.get("owned_scope")),
            ("ownership.description", descriptions),
            ("triggers.positive", _positive_triggers(skill)),
        )
    }


def _infer_legacy_role(boundary: Any) -> str | None:
    text = " ".join(_flatten_strings(boundary))
    if re.search(r"\b(primary|owns|owner|owned here)\b", text, re.I):
        return "primary"
    if re.search(r"\bfallback\b", text, re.I):
        return "fallback"
    if re.search(r"\b(defer|deferred|leave|handled by)\b", text, re.I):
        return "defer"
    return None


def _adapt_schema_1_0(plan_skills: dict[str, dict[str, Any]]) -> None:
    """Add internal 1.1-shaped metadata without changing the source plan."""
    for name, skill in plan_skills.items():
        if "_normalized_ownership" not in skill:
            paths = list(skill.get("writes", []))
            skill["_normalized_ownership"] = [
                {
                    "id": f"legacy:{name}",
                    "mode": "exclusive",
                    "description": " ".join(_flatten_strings(skill.get("owned_scope"))),
                    "paths": paths,
                }
            ]
        normalized_siblings = []
        for sibling in skill.get("nearest_siblings", []):
            if not isinstance(sibling, dict):
                continue
            normalized_siblings.append(
                {
                    **sibling,
                    "role": _infer_legacy_role(sibling.get("boundary")),
                    "ownership_ids": [f"legacy:{name}"],
                }
            )
        skill["_normalized_siblings"] = normalized_siblings


def _contract_projection(skill: dict[str, Any]) -> dict[str, tuple[list[str], list[str]]]:
    projection: dict[str, tuple[list[str], list[str]]] = {}
    for field in CONTRACT_PROJECTION_FIELDS:
        value = skill.get(field)
        if field == "triggers" and isinstance(value, dict):
            value = _as_list(value.get("positive")) + _as_list(value.get("negative"))
        lines = [
            normalized
            for raw in _flatten_strings(value)
            if (normalized := _normalize_line(raw))
        ]
        projection[field] = (lines, _tokens("\n".join(lines)))
    ownership_lines = [
        normalized
        for item in skill.get("_normalized_ownership", skill.get("ownership", []))
        if isinstance(item, dict)
        for raw in _flatten_strings(item.get("description"))
        if (normalized := _normalize_line(raw))
    ]
    projection["ownership"] = (
        ownership_lines,
        _tokens("\n".join(ownership_lines)),
    )
    boundary_lines = [
        normalized
        for item in skill.get("_normalized_siblings", skill.get("nearest_siblings", []))
        if isinstance(item, dict)
        for raw in _flatten_strings(item.get("boundary"))
        if (normalized := _normalize_line(raw))
    ]
    projection["nearest_siblings.boundary"] = (
        boundary_lines,
        _tokens("\n".join(boundary_lines)),
    )
    return projection


def _positive_triggers(skill: dict[str, Any]) -> list[str]:
    triggers = skill.get("triggers")
    if not isinstance(triggers, dict):
        return []
    return [
        item for item in _as_list(triggers.get("positive")) if isinstance(item, str)
    ]


def _routing_tokens(skill: dict[str, Any]) -> set[str]:
    return _meaningful_tokens(" ".join(_positive_triggers(skill)))


def _has_explicit_routing_precedence(
    left: dict[str, Any], right_name: str
) -> bool:
    siblings = left.get("_normalized_siblings", left.get("nearest_siblings", []))
    for sibling in siblings:
        if not isinstance(sibling, dict) or sibling.get("name") != right_name:
            continue
        if sibling.get("role") in ROUTING_ROLES:
            return True
        boundary = " ".join(_flatten_strings(sibling.get("boundary")))
        if re.search(r"\b(primary|defer|deferred|fallback|owns|owner)\b", boundary, re.I):
            return True
    return False


def _routing_relation(
    left: dict[str, Any], right_name: str
) -> dict[str, Any] | None:
    siblings = left.get("_normalized_siblings", left.get("nearest_siblings", []))
    return next(
        (
            sibling
            for sibling in siblings
            if isinstance(sibling, dict) and sibling.get("name") == right_name
        ),
        None,
    )


def _valid_reciprocal_roles(left: str | None, right: str | None) -> bool:
    return {left, right} in ({"primary", "defer"}, {"primary", "fallback"})


def _contract_ids(items: Any) -> set[str]:
    if not isinstance(items, list):
        return set()
    return {
        str(item["id"]).strip()
        for item in items
        if isinstance(item, dict) and _is_nonempty_string(item.get("id"))
    }


def _path_matches(target: Path, value: str) -> list[Path]:
    if any(token in value for token in "*?["):
        try:
            matches = sorted(target.glob(value))
        except (OSError, ValueError):
            return []
        confined: list[Path] = []
        for match in matches:
            try:
                relative = match.relative_to(target)
            except ValueError:
                continue
            candidate = _confined(target, str(relative))
            if candidate is not None and candidate.exists():
                confined.append(candidate)
        return confined
    candidate = _confined(target, value)
    return [candidate] if candidate is not None and candidate.exists() else []


def _creatable_parent_exists(target: Path, value: str) -> bool:
    prefix = _glob_prefix(value)
    candidate = _confined(target, prefix)
    if candidate is None:
        return False
    parent = candidate if value.endswith("/**") else candidate.parent
    return parent.is_dir()


def _symbol_identifier(symbol: str) -> str:
    """Last namespace/member segment of a qualified anchor symbol.

    ``EVIDENCE_ANCHOR_FORMAT`` admits qualified symbols (``Foo\\Bar::baz``,
    ``framework.messenger.transports``), so the resolvable unit is the final
    segment; the qualifiers name packages or config parents that need not
    occur verbatim in the cited file.
    """
    segments = [item for item in re.split(r"[.:\\-]", symbol) if item]
    return segments[-1] if segments else symbol


def _resolve_evidence_anchor(
    name: str,
    anchor: str,
    path_value: str,
    target: Path,
    diagnostics: list[Diagnostic],
) -> None:
    """Resolve a well-formed anchor against the file it cites.

    ``evidence[].line_range`` is bound to the real length of the cited file, and
    an anchor is the same kind of claim at skill granularity, so it is held to
    the same standard: a line range past EOF and a symbol that never occurs in
    the source both cite nothing. The source is read defensively - an
    unreadable file yields a diagnostic, never a traceback.
    """
    resolved = _confined(target, path_value)
    if resolved is None or not resolved.is_file():
        _diag(
            diagnostics,
            "EVIDENCE_ANCHOR_UNRESOLVABLE",
            f"{name} anchor cites a missing or unsafe source: {anchor}",
        )
        return
    try:
        text = resolved.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        _diag(
            diagnostics,
            "EVIDENCE_ANCHOR_UNRESOLVABLE",
            f"{name} anchor source cannot be read: {anchor} ({error})",
        )
        return
    body = anchor[len(path_value) + 1 :]
    if body.startswith("symbol:"):
        # Case-insensitive on purpose: PHP class/function names are
        # case-insensitive and config keys are re-cased freely, so only a
        # symbol that is absent under ANY casing is treated as unresolvable.
        identifier = _symbol_identifier(body[len("symbol:") :])
        if not re.search(
            rf"(?<![A-Za-z0-9_]){re.escape(identifier)}(?![A-Za-z0-9_])",
            text,
            re.I,
        ):
            _diag(
                diagnostics,
                "EVIDENCE_ANCHOR_SYMBOL_ABSENT",
                f"{name} anchor symbol does not occur in the cited source: {anchor}",
            )
        return
    try:
        bounds = [int(item.lstrip("L")) for item in body.split("-")]
    except ValueError:
        # Shape belongs to EVIDENCE_ANCHOR_FORMAT, which gates this call; a
        # malformed body reaching here is not re-reported and never raises.
        return
    start, end = bounds[0], bounds[-1]
    line_count = len(text.splitlines())
    if end < start or start > line_count or end > line_count:
        _diag(
            diagnostics,
            "EVIDENCE_ANCHOR_RANGE",
            f"{name} anchor line range falls outside the cited source "
            f"({line_count} lines): {anchor}",
        )


def _validate_schema_1_2_skill(
    name: str,
    skill: dict[str, Any],
    evidence_claims: dict[str, list[str]],
    evidence_map: dict[str, tuple[str, str]],
    target: Path,
    diagnostics: list[Diagnostic],
) -> None:
    try:
        command_analyzer: CommandAnalyzer | None = CommandAnalyzer(target)
    except CommandAnalysisError as error:
        command_analyzer = None
        _diag(
            diagnostics,
            "COMMAND_ANALYZER_UNAVAILABLE",
            f"{name} target commands cannot be analyzed safely: {error}",
        )
    capability = skill.get("capability")
    if (
        not isinstance(capability, dict)
        or set(capability) != {"mode", "summary"}
        or capability.get("mode") not in CAPABILITY_MODES
        or not _is_nonempty_string(capability.get("summary"))
    ):
        _diag(
            diagnostics,
            "SKILL_CAPABILITY_INVALID",
            f"{name}.capability must define mode and summary",
        )
        capability = {}
    elif capability["mode"] == "read-only":
        if skill.get("writes"):
            _diag(
                diagnostics,
                "READ_ONLY_WRITE_CONFLICT",
                f"{name} is read-only but declares writes",
            )
        procedure_language = " ".join(
            str(item.get("action", ""))
            for item in _as_list(skill.get("procedure_steps"))
            if isinstance(item, dict)
        )
        if READ_ONLY_MUTATION_PATTERN.search(procedure_language):
            _diag(
                diagnostics,
                "READ_ONLY_PROCEDURE_MUTATION",
                f"{name} read-only procedure contains write-oriented language",
            )
    elif capability.get("mode") == "workspace-write" and not skill.get("writes"):
        _diag(
            diagnostics,
            "WRITE_CAPABILITY_WITHOUT_PATH",
            f"{name} has workspace-write capability but declares no writes",
        )

    decision_ids: set[str] = set()
    for index, decision in enumerate(skill.get("decision_points", [])):
        if (
            not isinstance(decision, dict)
            or set(decision) != {"id", "question", "branches"}
            or not _is_nonempty_string(decision.get("id"))
            or not OWNERSHIP_ID_PATTERN.fullmatch(str(decision.get("id", "")))
            or not _is_nonempty_string(decision.get("question"))
            or not _is_string_list(decision.get("branches"))
        ):
            _diag(
                diagnostics,
                "DECISION_POINT_INVALID",
                f"{name}.decision_points[{index}] must be a typed schema 1.2 decision",
            )
            continue
        if decision["id"] in decision_ids:
            _diag(
                diagnostics,
                "DECISION_POINT_DUPLICATE",
                f"{name} repeats decision id: {decision['id']}",
            )
        decision_ids.add(decision["id"])

    procedure_steps = skill.get("procedure_steps")
    procedure_ids: set[str] = set()
    if not isinstance(procedure_steps, list) or not procedure_steps:
        _diag(
            diagnostics,
            "PROCEDURE_STEPS_INVALID",
            f"{name}.procedure_steps must be a non-empty array",
        )
        procedure_steps = []
    for index, step in enumerate(procedure_steps):
        if (
            not isinstance(step, dict)
            or set(step)
            != {
                "id",
                "action",
                "evidence_ids",
                "path_refs",
                "decision_refs",
                "expected_outcome",
                "failure_branch",
            }
            or not _is_nonempty_string(step.get("id"))
            or not OWNERSHIP_ID_PATTERN.fullmatch(str(step.get("id", "")))
            or not _is_nonempty_string(step.get("action"))
            or not _is_string_list(step.get("evidence_ids"), allow_empty=True)
            or not _is_string_list(step.get("path_refs"), allow_empty=True)
            or not _is_string_list(step.get("decision_refs"), allow_empty=True)
            or not _is_nonempty_string(step.get("expected_outcome"))
            or not _is_nonempty_string(step.get("failure_branch"))
        ):
            _diag(
                diagnostics,
                "PROCEDURE_STEP_INVALID",
                f"{name}.procedure_steps[{index}] has invalid typed fields",
            )
            continue
        if step["id"] in procedure_ids:
            _diag(
                diagnostics,
                "PROCEDURE_STEP_DUPLICATE",
                f"{name} repeats procedure step id: {step['id']}",
            )
        procedure_ids.add(step["id"])
        action = str(step["action"])
        action_hedge = _hedge_phrase(action)
        if action_hedge is not None:
            _diag(
                diagnostics,
                "PROCEDURE_STEP_NOT_OPERATIONAL",
                f"{name}.{step['id']} defers to reflection instead of an action "
                f"({action_hedge!r}): {_excerpt(action)}",
            )
        elif not _commands_action(action):
            _diag(
                diagnostics,
                "PROCEDURE_STEP_NOT_OPERATIONAL",
                f"{name}.{step['id']} commands no action: {_excerpt(action)}",
            )
        if (
            not step["path_refs"]
            and not step["evidence_ids"]
            and not _has_concrete_anchor(action)
        ):
            _diag(
                diagnostics,
                "PROCEDURE_STEP_UNANCHORED",
                f"{name}.{step['id']} names no path, evidence, or concrete anchor",
            )
        for evidence_id in step["evidence_ids"]:
            if evidence_id not in skill.get("evidence_ids", []):
                _diag(
                    diagnostics,
                    "PROCEDURE_EVIDENCE_UNKNOWN",
                    f"{name}.{step['id']} references undeclared evidence: {evidence_id}",
                )
        for path_ref in step["path_refs"]:
            if path_ref not in {
                item.get("path")
                for item in _as_list(skill.get("path_contracts"))
                if isinstance(item, dict)
            }:
                _diag(
                    diagnostics,
                    "PROCEDURE_PATH_UNKNOWN",
                    f"{name}.{step['id']} references an undeclared path: {path_ref}",
                )
        unknown_decisions = sorted(set(step["decision_refs"]) - decision_ids)
        if unknown_decisions:
            _diag(
                diagnostics,
                "PROCEDURE_DECISION_UNKNOWN",
                f"{name}.{step['id']} references unknown decisions: {unknown_decisions}",
            )

    verification = skill.get("verification")
    verification_ids: set[str] = set()
    if not isinstance(verification, list) or not verification:
        _diag(
            diagnostics,
            "VERIFICATION_CONTRACT_INVALID",
            f"{name}.verification must be a non-empty structured array",
        )
        verification = []
    for index, check in enumerate(verification):
        expected_fields = {
            "id",
            "mode",
            "instruction",
            "command",
            "prerequisites",
            "safe_scope",
            "mutation_class",
            "network_class",
            "expected_result",
            "failure_result",
            "skip_condition",
            "skip_reporting",
        }
        if (
            not isinstance(check, dict)
            or set(check) != expected_fields
            or not _is_nonempty_string(check.get("id"))
            or not OWNERSHIP_ID_PATTERN.fullmatch(str(check.get("id", "")))
            or check.get("mode") not in VERIFICATION_MODES
            or not _is_string_list(check.get("prerequisites"), allow_empty=True)
            or check.get("mutation_class") not in MUTATION_CLASSES
            or check.get("network_class") not in NETWORK_CLASSES
            or not all(
                _is_nonempty_string(check.get(field))
                for field in expected_fields
                - {
                    "id",
                    "mode",
                    "command",
                    "prerequisites",
                    "mutation_class",
                    "network_class",
                }
            )
            or not (
                (check.get("mode") == "command" and _is_nonempty_string(check.get("command")))
                or (check.get("mode") == "manual" and check.get("command") is None)
            )
        ):
            _diag(
                diagnostics,
                "VERIFICATION_CONTRACT_INVALID",
                f"{name}.verification[{index}] has invalid typed fields",
            )
            continue
        if check["id"] in verification_ids:
            _diag(
                diagnostics,
                "VERIFICATION_ID_DUPLICATE",
                f"{name} repeats verification id: {check['id']}",
            )
        verification_ids.add(check["id"])
        verification_text = " ".join(
            str(check[field])
            for field in (
                "instruction",
                "command",
                "safe_scope",
                "expected_result",
                "failure_result",
            )
            if check.get(field) is not None
        )
        if GENERIC_VERIFICATION_PATTERN.search(verification_text):
            _diag(
                diagnostics,
                "GENERIC_VERIFICATION",
                f"{name}.{check['id']} verification is generic rather than executable",
            )
        if NON_FALSIFIABLE_VERIFICATION_PATTERN.search(verification_text):
            _diag(
                diagnostics,
                "VERIFICATION_NOT_FALSIFIABLE",
                f"{name}.{check['id']} verification accepts an impression "
                "instead of a check",
            )
        command = str(check.get("command") or "")
        if (
            check.get("mutation_class") != "none"
            or re.search(r"(^|\s)--fix(?:\s|$)", command)
        ):
            _diag(
                diagnostics,
                "MUTATING_VERIFICATION_COMMAND",
                f"{name}.{check['id']} uses a mutating verification command",
            )
        if command and command_analyzer is not None:
            try:
                command_analysis = command_analyzer.analyze(
                    command, verification=True
                )
            except CommandAnalysisError as error:
                _diag(
                    diagnostics,
                    "COMMAND_ANALYSIS_FAILED",
                    f"{name}.{check['id']} command cannot be resolved: {error}",
                )
            else:
                if not command_analysis.verification_safe:
                    findings = ", ".join(
                        sorted({item.code for item in command_analysis.findings})
                    )
                    _diag(
                        diagnostics,
                        "COMMAND_RISK_BLOCKED",
                        f"{name}.{check['id']} is unsafe verification: {findings}",
                    )
                categories = set(command_analysis.categories)
                detected_network = (
                    "external-provider"
                    if "external_provider_network" in categories
                    else "none"
                )
                detected_mutation = (
                    "destructive"
                    if "destructive_database_deploy" in categories
                    else "workspace-write"
                    if "workspace_mutation" in categories
                    else "none"
                )
                if check.get("network_class") != detected_network:
                    _diag(
                        diagnostics,
                        "VERIFICATION_NETWORK_CLASS_MISMATCH",
                        f"{name}.{check['id']} declares {check.get('network_class')} "
                        f"but command analysis found {detected_network}",
                    )
                if check.get("mutation_class") != detected_mutation:
                    _diag(
                        diagnostics,
                        "VERIFICATION_MUTATION_CLASS_MISMATCH",
                        f"{name}.{check['id']} declares "
                        f"{check.get('mutation_class')} but command analysis found "
                        f"{detected_mutation}",
                    )
        if (
            check.get("network_class") == "external-provider"
            and capability.get("mode") != "external-side-effect"
        ):
            _diag(
                diagnostics,
                "VERIFICATION_NETWORK_CAPABILITY",
                f"{name}.{check['id']} declares provider network access without "
                "external-side-effect capability",
            )

    integration_safety = skill.get("integration_safety")
    safety_fields = {
        "network_policy",
        "test_double_strategy",
        "environment",
        "authorization_required",
        "rollback",
        "sanitization",
    }
    if (
        not isinstance(integration_safety, dict)
        or set(integration_safety) != safety_fields
        or integration_safety.get("network_policy") not in NETWORK_POLICIES
        or integration_safety.get("environment") not in INTEGRATION_ENVIRONMENTS
        or type(integration_safety.get("authorization_required")) is not bool
        or not all(
            _is_nonempty_string(integration_safety.get(field))
            for field in ("test_double_strategy", "rollback", "sanitization")
        )
    ):
        _diag(
            diagnostics,
            "INTEGRATION_SAFETY_INVALID",
            f"{name}.integration_safety has invalid typed fields",
        )
    elif str(skill.get("category", "")).lower() in INTEGRATION_CATEGORIES:
        policy = integration_safety["network_policy"]
        expected_environments = {
            "forbidden": {"none", "local"},
            "mock-only": {"none", "local"},
            "sandbox-with-approval": {"sandbox"},
            "approved-live": {"approved-live"},
        }
        if integration_safety["environment"] not in expected_environments[policy]:
            _diag(
                diagnostics,
                "PROVIDER_ENVIRONMENT_MISMATCH",
                f"{name} network policy and environment classification disagree",
            )
        if policy in {"sandbox-with-approval", "approved-live"} and not integration_safety[
            "authorization_required"
        ]:
            _diag(
                diagnostics,
                "PROVIDER_AUTHORIZATION_REQUIRED",
                f"{name} permits provider access without explicit authorization",
            )
        if policy == "approved-live" and not re.search(
            r"\b(fake|fixture|mock|stub|local|sandbox)\b",
            integration_safety["test_double_strategy"],
            re.I,
        ):
            _diag(
                diagnostics,
                "PROVIDER_SAFE_DEFAULT_MISSING",
                f"{name} lacks a fake, fixture, local adapter, or sandbox default",
            )
    if capability.get("mode") == "external-side-effect" and (
        not isinstance(integration_safety, dict)
        or integration_safety.get("network_policy")
        not in {"sandbox-with-approval", "approved-live"}
        or integration_safety.get("authorization_required") is not True
    ):
        _diag(
            diagnostics,
            "EXTERNAL_SIDE_EFFECT_UNAUTHORIZED",
            f"{name} external-side-effect capability requires an explicitly "
            "authorized sandbox or approved-live policy",
        )

    path_contracts = skill.get("path_contracts")
    declared_paths: set[str] = set()
    if not isinstance(path_contracts, list) or not path_contracts:
        _diag(
            diagnostics,
            "PATH_CONTRACTS_INVALID",
            f"{name}.path_contracts must be a non-empty array",
        )
        path_contracts = []
    for index, contract in enumerate(path_contracts):
        if (
            not isinstance(contract, dict)
            or set(contract) != {"path", "access", "classification", "evidence_ids"}
            or not _is_nonempty_string(contract.get("path"))
            or not _safe_relative(contract.get("path"))
            or _normalize_glob(contract["path"]) != contract["path"]
            or contract.get("access") not in PATH_ACCESS_MODES
            or contract.get("classification") not in PATH_CLASSIFICATIONS
            or not _is_string_list(contract.get("evidence_ids"), allow_empty=True)
        ):
            _diag(
                diagnostics,
                "PATH_CONTRACT_INVALID",
                f"{name}.path_contracts[{index}] has invalid typed fields",
            )
            continue
        path = contract["path"]
        declared_paths.add(path)
        if contract["access"] == "write" and path not in skill.get("writes", []):
            _diag(
                diagnostics,
                "PATH_WRITE_UNDECLARED",
                f"{name} write path contract is absent from writes: {path}",
            )
        if (
            contract["classification"] == "required-existing"
            and not _path_matches(target, path)
        ):
            _diag(
                diagnostics,
                "PATH_EXISTING_MISSING",
                f"{name} existing path does not resolve: {path}",
            )
        if (
            contract["classification"] == "generated-runtime"
            and str(skill.get("kind", "")).lower() != "runtime-fixed"
        ):
            _diag(
                diagnostics,
                "PATH_GENERATED_INVALID",
                f"{name} may classify generated paths only for runtime-fixed output: {path}",
            )
        if (
            contract["classification"] == "creatable"
            and (
                contract["access"] != "write"
                or (
                    str(skill.get("kind", "")).lower() != "runtime-fixed"
                    and not _creatable_parent_exists(target, path)
                )
            )
        ):
            _diag(
                diagnostics,
                "PATH_CREATABLE_INVALID",
                f"{name} creatable path must be writable beneath an existing parent: {path}",
            )
        for evidence_id in contract["evidence_ids"]:
            if evidence_id not in skill.get("evidence_ids", []):
                _diag(
                    diagnostics,
                    "PATH_EVIDENCE_UNKNOWN",
                    f"{name} path contract references undeclared evidence: {evidence_id}",
                )

    if str(skill.get("kind", "")).lower() == "runtime-fixed":
        try:
            runtime_document = json.loads(
                RUNTIME_CONTRACT.read_text(encoding="utf-8")
            )
            runtime_paths = runtime_document["path_contracts"]
            required_runtime = [
                str(item).rstrip("/") + (
                    "/**" if str(item).endswith("/") else ""
                )
                for item in runtime_paths["required_skeleton"]
            ]
            creatable_runtime = [
                str(item["path"]) for item in runtime_paths["creatable"]
            ]
            forbidden_runtime = [
                str(item) for item in runtime_paths["forbidden_invented_paths"]
            ]
        except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
            _diag(
                diagnostics,
                "RUNTIME_CONTRACT_UNREADABLE",
                f"canonical runtime contract is invalid: {error}",
            )
        else:
            for contract in path_contracts:
                if not isinstance(contract, dict) or "path" not in contract:
                    continue
                path = str(contract["path"])
                if any(_globs_intersect(path, item) for item in forbidden_runtime):
                    _diag(
                        diagnostics,
                        "RUNTIME_PATH_FORBIDDEN",
                        f"{name} references an invented runtime path: {path}",
                    )
                allowed = (
                    required_runtime
                    if contract.get("classification") == "generated-runtime"
                    else creatable_runtime
                    if contract.get("classification") == "creatable"
                    else []
                )
                if contract.get("classification") in {
                    "generated-runtime",
                    "creatable",
                } and not any(
                    _globs_intersect(path, item) for item in allowed
                ):
                    _diag(
                        diagnostics,
                        "RUNTIME_PATH_UNDECLARED",
                        f"{name} path is absent from the canonical runtime "
                        f"contract: {path}",
                    )
    for required_path in skill.get("source_paths", []) + skill.get("writes", []):
        if required_path not in declared_paths:
            _diag(
                diagnostics,
                "PATH_CONTRACT_MISSING",
                f"{name} has no path contract for: {required_path}",
            )

    anchors = skill.get("evidence_anchors")
    anchored_evidence: set[str] = set()
    if not isinstance(anchors, list) or (
        skill.get("evidence_ids") and not anchors
    ):
        _diag(
            diagnostics,
            "EVIDENCE_ANCHORS_INVALID",
            f"{name}.evidence_anchors must be a non-empty array",
        )
        anchors = []
    for index, anchor in enumerate(anchors):
        if (
            not isinstance(anchor, dict)
            or set(anchor)
            != {
                "evidence_id",
                "claim",
                "anchor",
                "procedure_step_ids",
                "verification_ids",
            }
            or anchor.get("evidence_id") not in skill.get("evidence_ids", [])
            or not _is_nonempty_string(anchor.get("claim"))
            or not _is_nonempty_string(anchor.get("anchor"))
            or not _is_string_list(anchor.get("procedure_step_ids"))
            or not _is_string_list(anchor.get("verification_ids"))
        ):
            _diag(
                diagnostics,
                "EVIDENCE_ANCHOR_INVALID",
                f"{name}.evidence_anchors[{index}] has invalid typed fields",
            )
            continue
        evidence_id = anchor["evidence_id"]
        anchored_evidence.add(evidence_id)
        if anchor["claim"] not in evidence_claims.get(evidence_id, []):
            _diag(
                diagnostics,
                "EVIDENCE_ANCHOR_CLAIM_UNKNOWN",
                f"{name} anchor claim is not declared by {evidence_id}",
            )
        if not set(anchor["procedure_step_ids"]) <= procedure_ids or not set(
            anchor["verification_ids"]
        ) <= verification_ids:
            _diag(
                diagnostics,
                "EVIDENCE_ANCHOR_REFERENCE_UNKNOWN",
                f"{name} anchor references unknown procedure or verification ids",
            )
        location = evidence_map.get(evidence_id)
        expected_prefix = location[1] if location and location[0] == "path" else ""
        if expected_prefix and not anchor["anchor"].startswith(expected_prefix + ":"):
            _diag(
                diagnostics,
                "EVIDENCE_ANCHOR_LOCATION",
                f"{name} anchor must begin with its canonical evidence path",
            )
        elif expected_prefix and not re.fullmatch(
            rf"{re.escape(expected_prefix)}:"
            r"(?:L[1-9][0-9]*(?:-L?[1-9][0-9]*)?|"
            r"symbol:[A-Za-z_][A-Za-z0-9_.:\\-]*)",
            anchor["anchor"],
        ):
            _diag(
                diagnostics,
                "EVIDENCE_ANCHOR_FORMAT",
                f"{name} anchor must use a bounded line range or stable symbol",
            )
        elif expected_prefix:
            _resolve_evidence_anchor(
                name, anchor["anchor"], expected_prefix, target, diagnostics
            )
        claim = anchor["claim"]
        step_text = " ".join(
            value
            for step in procedure_steps
            if isinstance(step, dict) and step.get("id") in anchor["procedure_step_ids"]
            for value in _flatten_strings(step)
        )
        verification_text = " ".join(
            value
            for check in verification
            if isinstance(check, dict) and check.get("id") in anchor["verification_ids"]
            for value in _flatten_strings(check)
        )
        if not all(
            _contract_matches(claim, text)
            for text in (
                " ".join(_flatten_strings(skill.get("owned_scope"))),
                step_text,
                verification_text,
                " ".join(_flatten_strings(skill.get("output_contract"))),
            )
        ):
            _diag(
                diagnostics,
                "CLAIM_TRACEABILITY_MISSING",
                f"{name} claim is not traceable through scope, procedure, verification, and output: {claim}",
            )
    for evidence_id in skill.get("evidence_ids", []):
        if evidence_id not in anchored_evidence:
            _diag(
                diagnostics,
                "EVIDENCE_ANCHOR_MISSING",
                f"{name} has no bounded anchor for evidence: {evidence_id}",
            )

    routing_cases = skill.get("routing_cases")
    if not isinstance(routing_cases, list) or not routing_cases:
        _diag(
            diagnostics,
            "ROUTING_CASES_INVALID",
            f"{name}.routing_cases must be a non-empty array",
        )
        routing_cases = []
    allowed_destinations = {name} | {
        sibling.get("name")
        for sibling in skill.get("nearest_siblings", [])
        if isinstance(sibling, dict)
    }
    destinations: set[str] = set()
    for index, case in enumerate(routing_cases):
        if (
            not isinstance(case, dict)
            or set(case)
            != {
                "prompt",
                "expected_primary",
                "permitted_secondary",
                "forbidden_skills",
                "rationale",
                "evidence_ids",
            }
            or not _is_nonempty_string(case.get("prompt"))
            or case.get("expected_primary") not in allowed_destinations
            or not _is_string_list(
                case.get("permitted_secondary"), allow_empty=True
            )
            or not _is_string_list(case.get("forbidden_skills"), allow_empty=True)
            or not _is_nonempty_string(case.get("rationale"))
            or not _is_string_list(case.get("evidence_ids"))
        ):
            _diag(
                diagnostics,
                "ROUTING_CASE_INVALID",
                f"{name}.routing_cases[{index}] has invalid typed fields",
            )
            continue
        if not set(case["permitted_secondary"]) <= allowed_destinations:
            _diag(
                diagnostics,
                "ROUTING_CASE_SECONDARY_UNKNOWN",
                f"{name} routing case permits a non-adjacent secondary owner",
            )
        if (
            case["expected_primary"] in case["permitted_secondary"]
            or set(case["permitted_secondary"]) & set(case["forbidden_skills"])
            or case["expected_primary"] in case["forbidden_skills"]
        ):
            _diag(
                diagnostics,
                "ROUTING_CASE_PRECEDENCE_CONFLICT",
                f"{name} routing case has contradictory owner precedence",
            )
        if not set(case["evidence_ids"]) <= set(skill.get("evidence_ids", [])):
            _diag(
                diagnostics,
                "ROUTING_CASE_EVIDENCE_UNKNOWN",
                f"{name} routing case references undeclared evidence",
            )
        destinations.add(case["expected_primary"])
        expected_contract = (
            skill.get("triggers", {}).get("positive", [])
            if case["expected_primary"] == name
            else skill.get("triggers", {}).get("negative", [])
        )
        if not _contract_matches(expected_contract, case["prompt"]):
            _diag(
                diagnostics,
                "ROUTING_CASE_TRIGGER_MISMATCH",
                f"{name} routing case does not match its expected positive or negative trigger",
            )
    required_destinations = {name} | (allowed_destinations - {name})
    if not required_destinations <= destinations:
        _diag(
            diagnostics,
            "ROUTING_CASE_COVERAGE",
            f"{name} routing cases do not cover self and every adjacent owner",
        )


def _validate_schema_1_2_plan_contracts(
    plan: dict[str, Any],
    plan_skills: dict[str, dict[str, Any]],
    evidence_map: dict[str, tuple[str, str]],
    diagnostics: list[Diagnostic],
) -> None:
    invariants = plan.get("critical_invariants")
    invariant_ids: set[str] = set()
    if not isinstance(invariants, list):
        _diag(
            diagnostics,
            "CRITICAL_INVARIANTS_INVALID",
            "critical_invariants must be an array",
        )
        invariants = []
    for index, invariant in enumerate(invariants):
        if (
            not isinstance(invariant, dict)
            or set(invariant)
            != {
                "id",
                "statement",
                "evidence_ids",
                "skill_names",
                "assertions",
            }
            or not _is_nonempty_string(invariant.get("id"))
            or not OWNERSHIP_ID_PATTERN.fullmatch(str(invariant.get("id", "")))
            or not _is_nonempty_string(invariant.get("statement"))
            or not _is_string_list(invariant.get("evidence_ids"))
            or not _is_string_list(invariant.get("skill_names"))
            or not isinstance(invariant.get("assertions"), list)
            or not invariant.get("assertions")
        ):
            _diag(
                diagnostics,
                "CRITICAL_INVARIANT_INVALID",
                f"critical_invariants[{index}] has invalid typed fields",
            )
            continue
        invariant_id = invariant["id"]
        if invariant_id in invariant_ids:
            _diag(
                diagnostics,
                "CRITICAL_INVARIANT_DUPLICATE",
                f"duplicate critical invariant id: {invariant_id}",
            )
        invariant_ids.add(invariant_id)
        valid_assertions: set[tuple[str, str]] = set()
        for assertion_index, assertion in enumerate(invariant["assertions"]):
            if (
                not isinstance(assertion, dict)
                or set(assertion) != {"skill_name", "verification_id"}
                or not _is_nonempty_string(assertion.get("skill_name"))
                or not _is_nonempty_string(assertion.get("verification_id"))
            ):
                _diag(
                    diagnostics,
                    "CRITICAL_INVARIANT_ASSERTION_INVALID",
                    f"{invariant_id}.assertions[{assertion_index}] is invalid",
                )
                continue
            valid_assertions.add(
                (assertion["skill_name"], assertion["verification_id"])
            )
        if not set(invariant["evidence_ids"]) <= set(evidence_map):
            _diag(
                diagnostics,
                "CRITICAL_INVARIANT_EVIDENCE_UNKNOWN",
                f"{invariant_id} references unknown evidence",
            )
        evidence_claim_text = " ".join(
            claim
            for skill in plan_skills.values()
            for anchor in _as_list(skill.get("evidence_anchors"))
            if isinstance(anchor, dict)
            and anchor.get("evidence_id") in invariant["evidence_ids"]
            for claim in [str(anchor.get("claim", ""))]
        )
        if not _contract_matches(invariant["statement"], evidence_claim_text):
            _diag(
                diagnostics,
                "CRITICAL_INVARIANT_CLAIM_TRACE",
                f"{invariant_id} is not traceable to an anchored evidence claim",
            )
        if not set(invariant["skill_names"]) <= set(plan_skills):
            _diag(
                diagnostics,
                "CRITICAL_INVARIANT_SKILL_UNKNOWN",
                f"{invariant_id} references unknown skills",
            )
        for skill_name in invariant["skill_names"]:
            skill = plan_skills.get(skill_name)
            if skill is None:
                continue
            if not set(invariant["evidence_ids"]) <= set(
                skill.get("evidence_ids", [])
            ):
                _diag(
                    diagnostics,
                    "CRITICAL_INVARIANT_SKILL_EVIDENCE",
                    f"{invariant_id} uses evidence not declared by {skill_name}",
                )
            verification_ids = {
                item.get("id")
                for item in skill.get("verification", [])
                if isinstance(item, dict)
            }
            assertion_ids = {
                verification_id
                for asserted_skill, verification_id in valid_assertions
                if asserted_skill == skill_name
            }
            if not assertion_ids or not assertion_ids <= verification_ids:
                _diag(
                    diagnostics,
                    "CRITICAL_INVARIANT_ASSERTION_MISSING",
                    f"{invariant_id} lacks a resolved regression assertion for "
                    f"{skill_name}",
                )
            procedure_text = " ".join(
                value
                for item in _as_list(skill.get("procedure_steps"))
                for value in _flatten_strings(item)
            )
            verification_text = " ".join(
                value
                for item in _as_list(skill.get("verification"))
                for value in _flatten_strings(item)
            )
            if not _contract_matches(invariant["statement"], procedure_text) or not _contract_matches(
                invariant["statement"], verification_text
            ):
                _diag(
                    diagnostics,
                    "CRITICAL_INVARIANT_COVERAGE",
                    f"{invariant_id} is not covered by {skill_name} procedure and verification",
                )

    for error in validate_plan_graph(plan):
        _diag(diagnostics, "FLOW_CONTRACT_INVALID", error)


def _validate_contract_inventory(
    plan_skills: dict[str, dict[str, Any]],
    diagnostics: list[Diagnostic],
    *,
    schema_version: str,
) -> None:
    """Validate the complete plan-level inventory exactly once per plan pass."""
    names = sorted(plan_skills)
    if schema_version == "1.0":
        _adapt_schema_1_0(plan_skills)

    if schema_version in {"1.1", "1.2"}:
        ownership_owners: defaultdict[str, set[str]] = defaultdict(set)
        ownership_entries: defaultdict[str, list[tuple[str, dict[str, Any]]]] = (
            defaultdict(list)
        )
        for name, skill in sorted(plan_skills.items()):
            ownership = skill.get("ownership")
            if not isinstance(ownership, list) or not ownership:
                _diag(
                    diagnostics,
                    "SKILL_OWNERSHIP_INVALID",
                    f"{name}.ownership must be a non-empty array",
                )
                continue
            local_ids: set[str] = set()
            for index, item in enumerate(ownership):
                if (
                    not isinstance(item, dict)
                    or set(item) != {"id", "mode", "description", "paths"}
                    or not _is_nonempty_string(item.get("id"))
                    or not OWNERSHIP_ID_PATTERN.fullmatch(
                        str(item.get("id", "")).strip()
                    )
                    or item.get("mode") not in OWNERSHIP_MODES
                    or not _is_nonempty_string(item.get("description"))
                    or not _is_string_list(item.get("paths"))
                ):
                    _diag(
                        diagnostics,
                        "SKILL_OWNERSHIP_INVALID",
                        f"{name}.ownership[{index}] must define id, mode, "
                        "description, and target-relative paths",
                    )
                    continue
                ownership_id = item["id"].strip()
                if ownership_id in local_ids:
                    _diag(
                        diagnostics,
                        "SKILL_OWNERSHIP_DUPLICATE",
                        f"{name} repeats ownership id: {ownership_id}",
                    )
                local_ids.add(ownership_id)
                ownership_owners[ownership_id].add(name)
                ownership_entries[ownership_id].append((name, item))
                for path in item["paths"]:
                    if not _safe_relative(path) or _normalize_glob(path) != path:
                        _diag(
                            diagnostics,
                            "SKILL_OWNERSHIP_PATH_INVALID",
                            f"{name}.{ownership_id} path is not normalized "
                            f"target-relative glob: {path}",
                        )
            skill["_normalized_ownership"] = ownership
            normalized_siblings = []
            for index, sibling in enumerate(skill.get("nearest_siblings", [])):
                if (
                    not isinstance(sibling, dict)
                    or set(sibling) != {"name", "role", "ownership_ids", "boundary"}
                    or sibling.get("role") not in ROUTING_ROLES
                    or not _is_string_list(sibling.get("ownership_ids"))
                    or not _is_nonempty_string(sibling.get("boundary"))
                ):
                    _diag(
                        diagnostics,
                        "SKILL_SIBLING_CONTRACT_INVALID",
                        f"{name}.nearest_siblings[{index}] must define name, role, "
                        "ownership_ids, and boundary",
                    )
                    continue
                normalized_siblings.append(sibling)
            skill["_normalized_siblings"] = normalized_siblings

        for ownership_id, entries in sorted(ownership_entries.items()):
            modes = {item["mode"] for _, item in entries}
            descriptions = {
                _normalize_line(item["description"]) for _, item in entries
            }
            if len(modes) != 1 or len(descriptions) != 1:
                _diag(
                    diagnostics,
                    "OWNERSHIP_ID_CONFLICT",
                    f"{ownership_id} has inconsistent mode or description "
                    f"across owners: {sorted(name for name, _ in entries)}",
                )
                continue
            mode = next(iter(modes))
            owners = sorted({name for name, _ in entries})
            if mode == "exclusive":
                if len(owners) != 1:
                    _diag(
                        diagnostics,
                        "OWNERSHIP_ID_CONFLICT",
                        f"exclusive ownership {ownership_id} has multiple "
                        f"owners: {owners}",
                    )
                # WRITE_SURFACE_COLLISION compares writes with writes, so a
                # read-only owner - whose writes are empty by contract - could
                # be written through by any neighbour. Exclusive means
                # exclusive: no non-owner may write the owned surface.
                owner_set = set(owners)
                owned_paths = sorted(
                    {
                        path
                        for _, item in entries
                        for path in item.get("paths", [])
                        if isinstance(path, str)
                    }
                )
                for writer_name in names:
                    if writer_name in owner_set:
                        continue
                    intrusions = sorted(
                        {
                            _normalize_glob(path)
                            for path in owned_paths
                            for write in _writes_of(plan_skills[writer_name])
                            if _globs_intersect(path, write)
                        }
                    )
                    if intrusions:
                        _diag(
                            diagnostics,
                            "OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT",
                            f"{writer_name} writes into exclusive ownership "
                            f"{ownership_id} held by {', '.join(owners)}: "
                            f"{', '.join(intrusions)}",
                        )
            if mode == "shared":
                for index, left_name in enumerate(owners):
                    for right_name in owners[index + 1 :]:
                        collisions = _write_collisions(
                            plan_skills[left_name].get("writes", []),
                            plan_skills[right_name].get("writes", []),
                        )
                        if collisions:
                            _diag(
                                diagnostics,
                                "OWNERSHIP_SHARED_WRITE_CONFLICT",
                                f"shared ownership {ownership_id} cannot authorize "
                                f"overlapping writes by {left_name} and {right_name}: "
                                f"{', '.join(sorted(collisions))}",
                            )
            if mode == "composed":
                paths = sorted(
                    {
                        path
                        for _, item in entries
                        for path in item.get("paths", [])
                    }
                )
                for path in paths:
                    writers = sorted(
                        {
                            name
                            for name, _ in entries
                            if any(
                                _globs_intersect(path, write)
                                for write in plan_skills[name].get("writes", [])
                            )
                        }
                    )
                    if len(writers) != 1:
                        _diag(
                            diagnostics,
                            "OWNERSHIP_COMPOSER_CONFLICT",
                            f"composed ownership {ownership_id} path {path} "
                            f"requires exactly one writer/composer; found {writers}",
                        )

        for name, skill in sorted(plan_skills.items()):
            for sibling in skill.get("_normalized_siblings", []):
                sibling_name = sibling["name"]
                if sibling_name not in plan_skills:
                    continue
                unknown = set(sibling["ownership_ids"]) - set(ownership_owners)
                if unknown:
                    _diag(
                        diagnostics,
                        "SKILL_SIBLING_OWNERSHIP_UNKNOWN",
                        f"{name} -> {sibling_name} references unknown ownership ids: "
                        f"{sorted(unknown)}",
                    )
                unrelated = sorted(
                    ownership_id
                    for ownership_id in sibling["ownership_ids"]
                    if ownership_id in ownership_owners
                    and not (
                        ownership_owners[ownership_id]
                        & {name, sibling_name}
                    )
                )
                if unrelated:
                    _diag(
                        diagnostics,
                        "SKILL_SIBLING_OWNERSHIP_MISMATCH",
                        f"{name} -> {sibling_name} references unrelated "
                        f"ownership ids: {unrelated}",
                    )
                reciprocal = _routing_relation(plan_skills[sibling_name], name)
                if reciprocal is None:
                    _diag(
                        diagnostics,
                        "SKILL_SIBLING_RECIPROCAL_MISSING",
                        f"{name} -> {sibling_name} has no reciprocal relation",
                    )
                    continue
                if set(reciprocal.get("ownership_ids", [])) != set(
                    sibling["ownership_ids"]
                ):
                    _diag(
                        diagnostics,
                        "SKILL_SIBLING_OWNERSHIP_MISMATCH",
                        f"{name} and {sibling_name} have different reciprocal ownership ids",
                    )
                if not _valid_reciprocal_roles(
                    sibling.get("role"), reciprocal.get("role")
                ):
                    _diag(
                        diagnostics,
                        "SKILL_SIBLING_ROLE_CONTRADICTION",
                        f"{name} and {sibling_name} require reciprocal "
                        "primary-defer or primary-fallback roles",
                    )

    projections = {
        name: _contract_projection(skill)
        for name, skill in sorted(plan_skills.items())
    }
    verbatim_fields = {
        name: _verbatim_boundary_fields(skill)
        for name, skill in sorted(plan_skills.items())
    }
    repeated_owners: defaultdict[tuple[str, str], set[str]] = defaultdict(set)
    for index, left_name in enumerate(names):
        left = plan_skills[left_name]
        left_scope = _scope_parts(left.get("owned_scope", []))
        for right_name in names[index + 1 :]:
            right = plan_skills[right_name]
            # A declared precedence resolves a PARTIAL overlap: the pair still
            # states which side owns what. It can never resolve text both
            # skills claim word for word - such a boundary divides nothing -
            # so this runs before and independently of the suppressors below.
            for field in (
                "owned_scope",
                "ownership.description",
                "triggers.positive",
            ):
                for line in sorted(
                    verbatim_fields[left_name][field]
                    & verbatim_fields[right_name][field]
                ):
                    _diag(
                        diagnostics,
                        "BOUNDARY_NOT_SEPARATING",
                        f"{left_name} and {right_name} declare the same "
                        f"{field} verbatim, so no declared precedence can "
                        f"separate them: {line}",
                    )
            collisions = _scopes_collide(
                left_scope, _scope_parts(right.get("owned_scope", []))
            )
            write_collisions = _write_collisions(
                left.get("writes", []), right.get("writes", [])
            )
            if collisions:
                relation = _routing_relation(left, right_name)
                reciprocal = _routing_relation(right, left_name)
                resolved = (
                    schema_version in {"1.1", "1.2"}
                    and relation is not None
                    and reciprocal is not None
                    and _valid_reciprocal_roles(
                        relation.get("role"), reciprocal.get("role")
                    )
                    and set(relation.get("ownership_ids", []))
                    == set(reciprocal.get("ownership_ids", []))
                )
                if not resolved:
                    _diag(
                        diagnostics,
                        "SCOPE_COLLISION",
                        f"{left_name} and {right_name} overlap ownership: "
                        f"{', '.join(sorted(collisions))}",
                    )
            if write_collisions:
                _diag(
                    diagnostics,
                    "WRITE_SURFACE_COLLISION",
                    f"{left_name} and {right_name} have intersecting writes: "
                    f"{', '.join(sorted(write_collisions))}",
                )
            left_routing = _routing_tokens(left)
            right_routing = _routing_tokens(right)
            routing_union = left_routing | right_routing
            routing_score = (
                len(left_routing & right_routing) / len(routing_union)
                if routing_union
                else 0.0
            )
            explicit_precedence = _has_explicit_routing_precedence(
                left, right_name
            ) or _has_explicit_routing_precedence(right, left_name)
            if routing_score >= 0.75 and not explicit_precedence:
                _diag(
                    diagnostics,
                    "ROUTING_AMBIGUITY",
                    f"{left_name} and {right_name} have overlapping positive "
                    f"routing ({routing_score:.3f}) without explicit precedence",
                )
            if not explicit_precedence:
                for left_trigger in _positive_triggers(left):
                    left_trigger_tokens = _meaningful_tokens(left_trigger)
                    if not left_trigger_tokens:
                        continue
                    for right_trigger in _positive_triggers(right):
                        right_trigger_tokens = _meaningful_tokens(right_trigger)
                        trigger_union = left_trigger_tokens | right_trigger_tokens
                        trigger_score = (
                            len(left_trigger_tokens & right_trigger_tokens)
                            / len(trigger_union)
                        )
                        if trigger_score >= 0.75:
                            _diag(
                                diagnostics,
                                "ROUTING_AMBIGUITY",
                                f"{left_name} and {right_name} share an "
                                f"ambiguous positive trigger "
                                f"({trigger_score:.3f}): {left_trigger!r} vs "
                                f"{right_trigger!r}",
                            )

            for field in CONTRACT_PROJECTION_FIELDS + (
                "ownership",
                "nearest_siblings.boundary",
            ):
                left_lines, left_tokens = projections[left_name][field]
                right_lines, right_tokens = projections[right_name][field]
                line_score = _line_similarity(left_lines, right_lines)
                token_score = _token_similarity(left_tokens, right_tokens)
                if (
                    len(left_tokens) >= 8
                    and len(right_tokens) >= 8
                    and (
                        line_score >= LINE_FAIL_THRESHOLD
                        or token_score >= TOKEN_WARN_THRESHOLD
                    )
                ):
                    _diag(
                        diagnostics,
                        "CONTRACT_SIMILARITY_WARN",
                        f"{left_name} and {right_name} have similar {field} "
                        f"contracts (line={line_score:.3f}, token={token_score:.3f})",
                        "warning",
                    )
                for line in set(left_lines) & set(right_lines):
                    if len(_tokens(line)) >= 8:
                        repeated_owners[(field, line)].update(
                            {left_name, right_name}
                        )
    for (field, line), owners in sorted(repeated_owners.items()):
        _diag(
            diagnostics,
            "CONTRACT_REPEATED_BLOCK",
            f"repeated {field} contract across {', '.join(sorted(owners))}: {line}",
            "warning",
        )


def _validate_plan(
    plan: dict[str, Any],
    plan_path: Path,
    target: Path,
    registry_path: Path,
    diagnostics: list[Diagnostic],
) -> tuple[dict[str, dict[str, Any]], dict[str, tuple[str, str]]]:
    schema_version = plan.get("schema_version")
    required_plan_fields = (
        SCHEMA_1_2_PLAN_FIELDS
        if schema_version == "1.2"
        else LEGACY_PLAN_FIELDS
    )
    for field in required_plan_fields:
        if field not in plan:
            _diag(diagnostics, "PLAN_FIELD_MISSING", f"plan missing top-level field: {field}")
    if diagnostics:
        return {}, {}
    extra_fields = set(plan) - set(required_plan_fields)
    if extra_fields:
        _diag(
            diagnostics,
            "PLAN_FIELD_UNKNOWN",
            f"plan contains unknown top-level fields: {sorted(extra_fields)}",
        )
    if schema_version not in SUPPORTED_PLAN_SCHEMAS:
        _diag(
            diagnostics,
            "PLAN_SCHEMA_VERSION",
            "schema_version must equal '1.0', '1.1', or '1.2'",
        )
    elif schema_version in {"1.0", "1.1"}:
        _diag(
            diagnostics,
            "PLAN_SCHEMA_MIGRATION",
            f"schema {schema_version} is accepted for audit diagnostics only; "
            "new plans must use schema 1.2",
            "warning",
        )
    if not _is_nonempty_string(plan["catalog_version"]):
        _diag(
            diagnostics,
            "PLAN_CATALOG_VERSION",
            "catalog_version must be a non-empty reference-corpus version",
        )
    registry = _load_registry(
        registry_path.expanduser().resolve(), plan["catalog_version"], diagnostics
    )
    if not _is_nonempty_string(plan["target_root"]):
        _diag(diagnostics, "PLAN_TARGET_ROOT", "target_root must be a non-empty string")
    else:
        root_value = Path(plan["target_root"]).expanduser()
        declared = (target / root_value).resolve() if not root_value.is_absolute() else root_value.resolve()
        if declared != target.resolve():
            _diag(diagnostics, "PLAN_TARGET_MISMATCH", f"target_root does not match --target: {plan['target_root']}")
    profile = plan["profile"]
    profile_rel = Path(profile) if _safe_relative(profile) else None
    profile_candidate = plan_path.parent / profile_rel.name if profile_rel else None
    profile_task_matches = bool(
        profile_rel
        and len(profile_rel.parts) == 3
        and profile_rel.parts[0] == "tasks"
        and profile_rel.parts[1] == plan_path.parent.name
        and profile_rel.parts[2] == "infra-scan-project-profile.md"
    )
    if (
        profile_candidate is None
        or not profile_task_matches
        or not profile_candidate.is_file()
    ):
        _diag(
            diagnostics,
            "PROFILE_INVALID",
            f"profile must resolve beside the generation plan: {profile!r}",
        )

    evidence_map: dict[str, tuple[str, str]] = {}
    evidence_claims: dict[str, list[str]] = {}
    evidence = plan["evidence"]
    if not isinstance(evidence, list) or not evidence:
        _diag(diagnostics, "EVIDENCE_EMPTY", "evidence must be a non-empty array")
    else:
        for index, entry in enumerate(evidence):
            if not isinstance(entry, dict) or not _is_nonempty_string(entry.get("id")):
                _diag(diagnostics, "EVIDENCE_INVALID", f"evidence[{index}] requires a non-empty id")
                continue
            evidence_id = entry["id"].strip()
            if evidence_id in evidence_map:
                _diag(diagnostics, "EVIDENCE_ID_DUPLICATE", f"duplicate evidence id: {evidence_id}")
                continue
            location = _evidence_location(entry)
            if location is None:
                _diag(diagnostics, "EVIDENCE_LOCATION", f"evidence {evidence_id} must define exactly one path or URL")
                continue
            kind, value = location
            allowed_evidence_fields = {
                "id",
                "path",
                "url",
                "source_type",
                "authority",
                "confidence",
                "line_range",
                "fingerprint",
                "supported_claims",
            }
            unknown_evidence_fields = set(entry) - allowed_evidence_fields
            if unknown_evidence_fields:
                _diag(
                    diagnostics,
                    "EVIDENCE_FIELD_UNKNOWN",
                    f"evidence {evidence_id} contains unknown fields: "
                    f"{sorted(unknown_evidence_fields)}",
                )
            for field in ("source_type", "authority", "confidence"):
                if not _is_nonempty_string(entry.get(field)):
                    _diag(
                        diagnostics,
                        "EVIDENCE_METADATA",
                        f"evidence {evidence_id} requires {field}",
                    )
            if entry.get("confidence") not in {"confirmed", "inferred", "unknown"}:
                _diag(
                    diagnostics,
                    "EVIDENCE_CONFIDENCE",
                    f"evidence {evidence_id} has invalid confidence",
                )
            if kind == "url":
                if not _approved_url(value):
                    _diag(diagnostics, "EVIDENCE_URL_INVALID", f"evidence {evidence_id} has an unapproved URL: {value}")
                    continue
            else:
                resolved = _confined(target, value)
                if resolved is None or not resolved.is_file():
                    _diag(diagnostics, "EVIDENCE_PATH_INVALID", f"evidence {evidence_id} path is unsafe or missing: {value}")
                    continue
                if re.search(r"(^|/)tasks/TASK-[^/]+/", value, re.I):
                    _diag(
                        diagnostics,
                        "EVIDENCE_TASK_PATH",
                        f"evidence {evidence_id} points to generator runtime: {value}",
                    )
                fingerprint = entry.get("fingerprint")
                actual = "sha256:" + hashlib.sha256(resolved.read_bytes()).hexdigest()
                if not _is_nonempty_string(fingerprint):
                    _diag(
                        diagnostics,
                        "EVIDENCE_FINGERPRINT_MISSING",
                        f"evidence {evidence_id} requires a sha256 fingerprint",
                    )
                elif fingerprint != actual:
                    _diag(
                        diagnostics,
                        "EVIDENCE_FINGERPRINT",
                        f"evidence {evidence_id} fingerprint drifted",
                    )
                line_range = entry.get("line_range")
                if line_range is not None and (
                    not isinstance(line_range, dict)
                    or set(line_range) != {"start", "end"}
                    or type(line_range.get("start")) is not int
                    or type(line_range.get("end")) is not int
                    or line_range["start"] < 1
                    or line_range["end"] < line_range["start"]
                ):
                    _diag(
                        diagnostics,
                        "EVIDENCE_LINE_RANGE",
                        f"evidence {evidence_id} has an invalid line_range",
                    )
                    line_range = None
                elif line_range is not None:
                    line_count = len(
                        resolved.read_text(
                            encoding="utf-8", errors="replace"
                        ).splitlines()
                    )
                    if line_range["end"] > line_count:
                        _diag(
                            diagnostics,
                            "EVIDENCE_LINE_RANGE",
                            f"evidence {evidence_id} line_range exceeds file length",
                        )
            if not _is_string_list(entry.get("supported_claims")):
                _diag(
                    diagnostics,
                    "EVIDENCE_CLAIMS",
                    f"evidence {evidence_id} requires supported_claims",
                )
            else:
                evidence_claims[evidence_id] = entry["supported_claims"]
                if kind == "path" and resolved is not None and resolved.is_file():
                    lines = resolved.read_text(
                        encoding="utf-8", errors="replace"
                    ).splitlines()
                    cited_text = "\n".join(lines)
                    if isinstance(line_range, dict):
                        start = max(0, line_range.get("start", 1) - 1)
                        end = min(len(lines), line_range.get("end", 0))
                        cited_text = "\n".join(lines[start:end])
                    cited_tokens = _meaningful_tokens(cited_text)
                    for claim in entry["supported_claims"]:
                        claim_tokens = _meaningful_tokens(claim) - {
                            "project", "confirmed", "supports", "uses", "runtime"
                        }
                        required = 1 if len(claim_tokens) <= 3 else 2
                        if len(claim_tokens & cited_tokens) < required:
                            _diag(
                                diagnostics,
                                "EVIDENCE_CLAIM_UNSUPPORTED",
                                f"evidence {evidence_id} claim is not grounded "
                                f"in the cited source/range: {claim}",
                            )
            evidence_map[evidence_id] = location

    plan_skills: dict[str, dict[str, Any]] = {}
    rejected = plan["rejected_candidates"]
    rejected_names: set[str] = set()
    rejected_candidate_ids: list[str] = []
    if not isinstance(rejected, list):
        _diag(
            diagnostics,
            "REJECTED_CANDIDATES_INVALID",
            "rejected_candidates must be an array",
        )
    else:
        for index, item in enumerate(rejected):
            if not isinstance(item, dict) or set(item) != {
                "candidate_id", "name", "category", "reason", "missing_evidence"
            }:
                _diag(
                    diagnostics,
                    "REJECTED_CANDIDATE_INVALID",
                    f"rejected_candidates[{index}] has invalid fields",
                )
            elif (
                not all(
                    _is_nonempty_string(item.get(field))
                    for field in ("candidate_id", "name", "category", "reason")
                )
                or not _is_string_list(item.get("missing_evidence"), allow_empty=True)
            ):
                _diag(
                    diagnostics,
                    "REJECTED_CANDIDATE_INVALID",
                    f"rejected_candidates[{index}] is incomplete",
                )
            elif item["name"] in rejected_names:
                _diag(
                    diagnostics,
                    "REJECTED_CANDIDATE_DUPLICATE",
                    f"duplicate rejected candidate: {item['name']}",
                )
            else:
                rejected_names.add(item["name"])
                rejected_candidate_ids.append(item["candidate_id"])
                registry_candidate = registry.get(item["candidate_id"])
                if registry_candidate is None:
                    _diag(
                        diagnostics,
                        "REJECTED_CANDIDATE_UNKNOWN",
                        f"rejected candidate is absent from registry: "
                        f"{item['candidate_id']}",
                    )
                elif (
                    item["category"].lower()
                    != registry_candidate["category"].lower()
                    or (
                        registry_candidate["mode"] == "static"
                        and item["name"] != item["candidate_id"]
                    )
                    or registry_candidate["mode"] == "runtime-fixed"
                    or not item["missing_evidence"]
                ):
                    _diag(
                        diagnostics,
                        "REJECTED_CANDIDATE_CONTRACT",
                        f"rejected candidate does not satisfy registry disposition "
                        f"rules: {item['candidate_id']}",
                    )
    skills = plan["skills"]
    if not isinstance(skills, list) or not skills:
        _diag(diagnostics, "SKILL_PLAN_EMPTY", "skills must be a non-empty array")
        return {}, evidence_map
    for index, skill in enumerate(skills):
        if not isinstance(skill, dict):
            _diag(diagnostics, "SKILL_PLAN_INVALID", f"skills[{index}] must be an object")
            continue
        required_skill_fields = (
            SCHEMA_1_2_SKILL_FIELDS
            if schema_version == "1.2"
            else REQUIRED_SKILL_FIELDS
        )
        missing = [field for field in required_skill_fields if field not in skill]
        if missing:
            _diag(diagnostics, "SKILL_PLAN_INCOMPLETE", f"skills[{index}] missing: {', '.join(missing)}")
            continue
        name = skill.get("name")
        if not _is_nonempty_string(name):
            _diag(diagnostics, "SKILL_NAME_INVALID", f"skills[{index}] has an invalid name")
            continue
        name = name.strip()
        if name in plan_skills:
            _diag(diagnostics, "SKILL_NAME_DUPLICATE", f"duplicate skill name: {name}")
            continue
        structural = True
        scalar_fields = ("category", "kind", "phase", "necessity_rationale")
        for field in scalar_fields:
            if not _is_nonempty_string(skill[field]):
                _diag(diagnostics, "SKILL_PLAN_VALUE", f"{name}.{field} must be substantive")
        triggers = skill["triggers"]
        if (
            not isinstance(triggers, dict)
            or not _is_string_list(triggers.get("positive"))
            or not _is_string_list(triggers.get("negative"))
        ):
            _diag(diagnostics, "SKILL_TRIGGERS_INVALID", f"{name}.triggers requires positive and negative string arrays")
            if not isinstance(triggers, dict) or not all(
                isinstance(triggers.get(key), list)
                for key in ("positive", "negative")
            ):
                structural = False
        runtime_fixed = str(skill.get("kind", "")).lower() == "runtime-fixed"
        for field in ("evidence_ids", "source_paths", "writes", "related_skills"):
            allow_empty = field == "writes" or (runtime_fixed and field in {"evidence_ids", "source_paths"})
            if not _is_string_list(skill[field], allow_empty=allow_empty):
                _diag(diagnostics, "SKILL_PLAN_VALUE", f"{name}.{field} must be a string array")
                if not _is_string_list(skill[field], allow_empty=True):
                    structural = False
        for field in (
            "owned_scope", "excluded_scope", "required_procedure_roles",
            "decision_points", "verification", "output_contract",
            "failure_handling", "nearest_siblings",
        ):
            if not _is_substantive_contract(skill[field]):
                _diag(diagnostics, "SKILL_PLAN_VALUE", f"{name}.{field} must be a substantive array")
                if not isinstance(skill[field], (str, list)):
                    structural = False
        fixed_blocks = skill.get("fixed_blocks", [])
        if not isinstance(fixed_blocks, list) or len(
            _fixed_block_contents(fixed_blocks)
        ) != len(fixed_blocks):
            _diag(
                diagnostics,
                "SKILL_FIXED_BLOCKS_INVALID",
                f"{name}.fixed_blocks must contain versioned id/version/content objects",
            )
        selection_gate = skill.get("selection_gate")
        if not isinstance(selection_gate, dict) or set(selection_gate) != {
            "catalog",
            "candidate_id",
            "candidate",
            "conditions",
            "distinct_value_from",
        }:
            _diag(
                diagnostics,
                "SKILL_SELECTION_GATE",
                f"{name}.selection_gate has invalid fields",
            )
            if not isinstance(selection_gate, dict):
                structural = False
        elif structural:
            candidate_id = selection_gate.get("candidate_id")
            registry_candidate = (
                registry.get(candidate_id)
                if _is_nonempty_string(candidate_id)
                else None
            )
            if registry_candidate is None:
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_CANDIDATE_UNKNOWN",
                    f"{name} references unknown registry candidate",
                )
            elif (
                selection_gate.get("catalog") != registry_candidate["catalog"]
                or str(skill.get("category", "")).lower()
                != registry_candidate["category"].lower()
                or (
                    registry_candidate["mode"] in {"static", "runtime-fixed"}
                    and selection_gate.get("candidate_id") != name
                )
                or (
                    registry_candidate["mode"] == "runtime-fixed"
                    and not runtime_fixed
                )
            ):
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_REGISTRY_MISMATCH",
                    f"{name} selection gate does not match candidate registry",
                )
            if (
                not _is_nonempty_string(selection_gate.get("catalog"))
                or not _is_nonempty_string(selection_gate.get("candidate_id"))
                or selection_gate.get("candidate") != name
                or not _is_string_list(
                    selection_gate.get("distinct_value_from"), allow_empty=True
                )
                or not isinstance(selection_gate.get("conditions"), list)
                or not selection_gate["conditions"]
            ):
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_GATE",
                    f"{name}.selection_gate is incomplete",
                )
            else:
                for condition in selection_gate["conditions"]:
                    if (
                        not isinstance(condition, dict)
                        or set(condition)
                        != {"requirement", "evidence_ids", "status", "explanation"}
                        or not _is_nonempty_string(condition.get("requirement"))
                        or condition.get("status") != "satisfied"
                        or not _is_nonempty_string(condition.get("explanation"))
                        or not _is_string_list(
                            condition.get("evidence_ids"),
                            allow_empty=runtime_fixed,
                        )
                    ):
                        _diag(
                            diagnostics,
                            "SKILL_SELECTION_CONDITION",
                            f"{name} has an unsatisfied/incomplete selection condition",
                        )
                        continue
                    for condition_evidence in condition.get("evidence_ids", []):
                        if condition_evidence not in skill.get("evidence_ids", []):
                            _diag(
                                diagnostics,
                                "SKILL_SELECTION_EVIDENCE",
                                f"{name} selection condition uses undeclared evidence: "
                                f"{condition_evidence}",
                            )
                    claims = [
                        claim
                        for condition_evidence in condition.get("evidence_ids", [])
                        for claim in evidence_claims.get(condition_evidence, [])
                    ]
                    if claims and not _contract_matches(
                        claims,
                        condition.get("requirement", "")
                        + " "
                        + condition.get("explanation", "")
                        + " "
                        + str(skill.get("necessity_rationale", "")),
                    ):
                        _diag(
                            diagnostics,
                            "SKILL_SELECTION_CLAIM_TRACE",
                            f"{name} selection condition is not supported by cited claims",
                        )
        if not structural:
            continue
        for evidence_id in skill.get("evidence_ids", []):
            if evidence_id not in evidence_map:
                _diag(diagnostics, "SKILL_EVIDENCE_UNKNOWN", f"{name} references unknown evidence: {evidence_id}")
        for source in skill.get("source_paths", []):
            resolved = _confined(target, source)
            if resolved is None or not resolved.exists():
                _diag(diagnostics, "SKILL_SOURCE_INVALID", f"{name} source path is unsafe or missing: {source}")
            if re.search(r"(^|/)tasks/TASK-[^/]+/", source, re.I):
                _diag(
                    diagnostics,
                    "SKILL_SOURCE_TASK_PATH",
                    f"{name} source path points to generator runtime: {source}",
                )
        for write in skill.get("writes", []):
            if not _safe_relative(write):
                _diag(diagnostics, "SKILL_WRITE_INVALID", f"{name} write path must be target-relative: {write}")
            elif _normalize_glob(write) != write:
                _diag(
                    diagnostics,
                    "SKILL_WRITE_INVALID",
                    f"{name} write glob must be normalized: {write}",
                )
        if schema_version == "1.2":
            _validate_schema_1_2_skill(
                name,
                skill,
                evidence_claims,
                evidence_map,
                target,
                diagnostics,
            )
            if not all(
                isinstance(skill.get(field), list)
                for field in (
                    "procedure_steps",
                    "path_contracts",
                    "evidence_anchors",
                    "routing_cases",
                )
            ):
                continue
        plan_skills[name] = skill

    names = set(plan_skills)
    selected_candidate_ids = [
        skill.get("selection_gate", {}).get("candidate_id")
        for skill in plan_skills.values()
        if _is_nonempty_string(
            skill.get("selection_gate", {}).get("candidate_id")
        )
    ]
    coverage_counts = Counter(selected_candidate_ids + rejected_candidate_ids)
    dual_disposition = set(selected_candidate_ids) & set(rejected_candidate_ids)
    if dual_disposition:
        _diag(
            diagnostics,
            "INVENTORY_CANDIDATE_DUAL_DISPOSITION",
            f"candidate families cannot be both selected and rejected: "
            f"{sorted(dual_disposition)}",
        )
    for candidate_id, candidate in sorted(registry.items()):
        count = coverage_counts.get(candidate_id, 0)
        if count == 0:
            _diag(
                diagnostics,
                "INVENTORY_CANDIDATE_UNACCOUNTED",
                f"registry candidate is neither selected nor rejected: {candidate_id}",
            )
        elif candidate["mode"] != "family" and count != 1:
            _diag(
                diagnostics,
                "INVENTORY_CANDIDATE_DUPLICATE",
                f"non-family candidate appears {count} times: {candidate_id}",
            )
    for candidate_id in sorted(set(coverage_counts) - set(registry)):
        _diag(
            diagnostics,
            "INVENTORY_CANDIDATE_UNKNOWN",
            f"plan accounts for unknown registry candidate: {candidate_id}",
        )
    overlap = names & rejected_names
    if overlap:
        _diag(
            diagnostics,
            "INVENTORY_SELECTED_AND_REJECTED",
            f"candidates cannot be selected and rejected: {sorted(overlap)}",
        )
    for name, skill in sorted(plan_skills.items()):
        siblings = list(skill.get("related_skills", [])) + [
            sibling.get("name")
            for sibling in skill.get("nearest_siblings", [])
            if isinstance(sibling, dict)
        ]
        for sibling in siblings:
            if sibling == name or sibling not in names:
                _diag(diagnostics, "SKILL_SIBLING_UNKNOWN", f"{name} has unresolved sibling reference: {sibling}")
        for candidate in _as_list(
            skill.get("selection_gate", {}).get("distinct_value_from")
        ):
            if candidate not in names | rejected_names:
                _diag(
                    diagnostics,
                    "SKILL_SELECTION_BOUNDARY_UNKNOWN",
                    f"{name} distinguishes unknown candidate: {candidate}",
                )
    if schema_version == "1.2":
        _validate_schema_1_2_plan_contracts(
            plan,
            plan_skills,
            evidence_map,
            diagnostics,
        )
    if schema_version in SUPPORTED_PLAN_SCHEMAS:
        _validate_contract_inventory(
            plan_skills,
            diagnostics,
            schema_version=schema_version,
        )
    return plan_skills, evidence_map


def _validate_skill_file(
    name: str,
    plan: dict[str, Any],
    path: Path,
    evidence_map: dict[str, tuple[str, str]],
    target: Path,
    diagnostics: list[Diagnostic],
) -> tuple[str, list[str], list[str]]:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        _diag(diagnostics, "SKILL_UNREADABLE", f"{name}: cannot read SKILL.md: {error}")
        return "", [], []
    frontmatter, body = _parse_frontmatter(text)
    _validate_body_commands(name, body, target, diagnostics)
    if frontmatter.get("name") != name:
        _diag(diagnostics, "SKILL_FRONTMATTER_NAME", f"{name}: directory and frontmatter name differ")
    description = frontmatter.get("description", "")
    if not _is_nonempty_string(description) or not re.search(r"\b(when|use|trigger|for)\b", description, re.I):
        _diag(diagnostics, "SKILL_DESCRIPTION", f"{name}: description must identify when the skill is selected")

    sections = _sections(body)
    resolved: dict[str, str] = {}
    insubstantial_sections = 0
    for role, aliases in SECTION_ALIASES.items():
        content = _section(sections, aliases)
        resolved[role] = content
        if len(_meaningful_tokens(content)) < 3:
            insubstantial_sections += 1
            _diag(diagnostics, "SKILL_SECTION_MISSING", f"{name}: missing or insubstantial {role} section")

    lower = body.lower()
    triggers = plan.get("triggers", {})
    if not _contract_matches(triggers.get("positive"), description + "\n" + body):
        _diag(
            diagnostics,
            "SKILL_POSITIVE_TRIGGER_TRACE",
            f"{name}: positive routing trigger is not traceable",
        )
    if not _contract_matches(triggers.get("negative"), body):
        _diag(
            diagnostics,
            "SKILL_NEGATIVE_TRIGGER_TRACE",
            f"{name}: negative routing/deferral is not traceable",
        )
    for phrase in GENERIC_PHRASES:
        if phrase in lower:
            _diag(diagnostics, "SKILL_GENERIC_PHRASE", f"{name}: generic phrase is not operational: {phrase!r}")
    # Naming the project is not instructing the agent: every step must command
    # an action, and the procedure as a whole must name something concrete.
    procedure_steps = _procedure_steps(resolved["procedure"])
    for index, step in enumerate(procedure_steps, 1):
        hedge = _hedge_phrase(step)
        if hedge is not None:
            _diag(
                diagnostics,
                "SKILL_STEP_HEDGED",
                f"{name}: procedure step {index} defers to reflection instead of "
                f"an action ({hedge!r}): {_excerpt(step)}",
            )
        elif not _commands_action(step):
            _diag(
                diagnostics,
                "SKILL_STEP_NOT_OPERATIONAL",
                f"{name}: procedure step {index} commands no action: {_excerpt(step)}",
            )
    # Anchors are counted over the step text only: the "1." list markers are
    # not evidence that the procedure names anything.
    if procedure_steps and not _has_concrete_anchor(" ".join(procedure_steps)):
        _diag(
            diagnostics,
            "SKILL_PROCEDURE_UNANCHORED",
            f"{name}: procedure names no concrete anchor (path, symbol, command, "
            "constant, or number)",
        )
    if NON_FALSIFIABLE_VERIFICATION_PATTERN.search(resolved["verification"]):
        _diag(
            diagnostics,
            "SKILL_VERIFICATION_NOT_FALSIFIABLE",
            f"{name}: verification accepts an impression instead of a check",
        )
    elif resolved["verification"] and not (
        _has_action_verb(resolved["verification"], VERIFICATION_VERB_FORMS)
        or _prescribes_tooling(resolved["verification"])
    ):
        _diag(
            diagnostics,
            "SKILL_VERIFICATION_NOT_OPERATIONAL",
            f"{name}: verification names no check to run: "
            f"{_excerpt(resolved['verification'])}",
        )
    if re.search(rf"\b{_name_pattern(name)}\b.*\b{_name_pattern(name)}\b", resolved["purpose"], re.I):
        _diag(diagnostics, "SKILL_CIRCULAR_PURPOSE", f"{name}: purpose is circular")

    checks = (
        ("necessity_rationale", body, "SKILL_RATIONALE_TRACE"),
        ("required_procedure_roles", resolved["procedure"], "SKILL_PROCEDURE_TRACE"),
        ("decision_points", resolved["procedure"], "SKILL_DECISION_TRACE"),
        ("verification", resolved["verification"], "SKILL_VERIFICATION_TRACE"),
        ("output_contract", resolved["outputs"], "SKILL_OUTPUT_TRACE"),
        ("failure_handling", resolved["failure"], "SKILL_FAILURE_TRACE"),
        ("owned_scope", body, "SKILL_OWNED_SCOPE_TRACE"),
        ("excluded_scope", body, "SKILL_EXCLUDED_SCOPE_TRACE"),
    )
    for field, haystack, code in checks:
        if not _contract_matches(plan[field], haystack):
            _diag(diagnostics, code, f"{name}: {field} is not traceable to skill content")

    expected_refs: list[str] = list(plan.get("source_paths", []))
    for evidence_id in plan.get("evidence_ids", []):
        location = evidence_map.get(evidence_id)
        if location and location[0] == "path":
            expected_refs.append(location[1])
    kind = str(plan.get("kind", "")).lower()
    if kind in TARGET_DERIVED_KINDS or expected_refs:
        if not any(reference.lower() in lower for reference in expected_refs):
            _diag(diagnostics, "SKILL_TARGET_REFERENCE", f"{name}: no concrete target evidence path appears in the skill")
    if kind == "runtime-fixed" and not re.search(
        r"\b(memory-bank|project-brain)/", body
    ):
        _diag(
            diagnostics,
            "SKILL_RUNTIME_REFERENCE",
            f"{name}: runtime-fixed skill must cite its generated target-relative runtime",
        )

    category = str(plan.get("category", "")).lower()
    if category in INTEGRATION_CATEGORIES or category in DOMAIN_CATEGORIES:
        evidence_chunks: list[str] = []
        for evidence_id in plan.get("evidence_ids", []):
            if evidence_id not in evidence_map:
                continue
            evidence_kind, evidence_value = evidence_map[evidence_id]
            evidence_chunks.append(evidence_value)
            evidence_path = _confined(target, evidence_value) if evidence_kind == "path" else None
            if evidence_path and evidence_path.is_file():
                try:
                    evidence_chunks.append(evidence_path.read_text(encoding="utf-8")[:200_000])
                except (OSError, UnicodeError):
                    pass
        evidence_text = " ".join(evidence_chunks)
        identifying = _meaningful_tokens(name.replace("-", " ")) - IDENTITY_STOPWORDS
        if identifying and not identifying.intersection(_meaningful_tokens(evidence_text)):
            _diag(diagnostics, "SKILL_EVIDENCE_IRRELEVANT", f"{name}: evidence does not identify the planned concern")
    if category in INTEGRATION_CATEGORIES:
        identity = set(_meaningful_tokens(name.replace("-", " ")) | _meaningful_tokens(evidence_text))
        if not identity.intersection(_meaningful_tokens(body)):
            _diag(diagnostics, "INTEGRATION_IDENTITY", f"{name}: integration identity is not concrete")
        if not re.search(r"\b(runtime|boundary|client|adapter|service|transport|sdk|api)\b", body, re.I):
            _diag(diagnostics, "INTEGRATION_BOUNDARY", f"{name}: integration runtime boundary is missing")
    if category in DOMAIN_CATEGORIES:
        if not re.search(r"\b(invariant|transition|permission|role|failure|forbidden|state)\b", body, re.I):
            _diag(diagnostics, "DOMAIN_CONTRACT", f"{name}: domain invariants, transitions, permissions, or failures are missing")

    siblings = list(plan.get("related_skills", [])) + [
        sibling.get("name")
        for sibling in plan.get("nearest_siblings", [])
        if isinstance(sibling, dict)
    ]
    for sibling in siblings:
        if not isinstance(sibling, str) or sibling.lower() not in lower:
            _diag(diagnostics, "SKILL_SIBLING_TRACE", f"{name}: sibling {sibling} is not referenced in skill content")
    for sibling in plan.get("nearest_siblings", []):
        if isinstance(sibling, dict) and not _contract_matches(
            sibling.get("boundary"), body
        ):
            _diag(
                diagnostics,
                "SKILL_SIBLING_BOUNDARY_TRACE",
                f"{name}: nearest-sibling boundary is not traceable",
            )

    fixed_blocks = _fixed_block_contents(plan.get("fixed_blocks", []))
    for block in fixed_blocks:
        if block not in body:
            _diag(diagnostics, "FIXED_BLOCK_MISSING", f"{name}: approved fixed block is not an exact body block")
        elif block not in resolved["failure"]:
            _diag(diagnostics, "FIXED_BLOCK_SCOPE", f"{name}: fixed block must be confined to failure handling")
        if len(_tokens(block)) > 120:
            _diag(diagnostics, "FIXED_BLOCK_OVERSIZED", f"{name}: fixed block is too large for a shared exemption")
    normalized_lines, normalized_tokens = _normalized_body(body, fixed_blocks)
    if (
        insubstantial_sections >= 3
        or len(set(normalized_lines)) < 5
        or len(set(normalized_tokens)) < 18
    ):
        _diag(diagnostics, "SKILL_CONTENT_EMPTY", f"{name}: substantive content is empty or heavily paraphrased filler")
    return body, normalized_lines, normalized_tokens


def _validate_authored_inventory(
    skills_dir: Path,
    plan_skills: dict[str, dict[str, Any]],
    evidence_map: dict[str, tuple[str, str]],
    target: Path,
    diagnostics: list[Diagnostic],
    *,
    allow_partial_skills: bool,
) -> None:
    """Validate existing SKILL.md bodies; plan contracts are already validated."""
    actual: dict[str, Path] = {}
    for child in sorted(skills_dir.iterdir(), key=lambda item: item.name):
        if child.is_symlink():
            _diag(diagnostics, "SKILL_PATH_UNSAFE", f"skill path must not be a symlink: {child.name}")
        elif child.is_dir() and (child / "SKILL.md").is_file():
            actual[child.name] = child / "SKILL.md"
    if not allow_partial_skills:
        for name in sorted(set(plan_skills) - set(actual)):
            _diag(diagnostics, "SKILL_FILE_MISSING", f"planned skill has no SKILL.md: {name}")
    for name in sorted(set(actual) - set(plan_skills)):
        _diag(diagnostics, "SKILL_UNPLANNED", f"skill has no plan entry: {name}")

    normalized: dict[str, tuple[list[str], list[str]]] = {}
    skeletons: dict[str, list[str]] = {}
    entities = frozenset(set(plan_skills) | set(actual))
    for name in sorted(set(plan_skills) & set(actual)):
        body, lines, tokens = _validate_skill_file(
            name, plan_skills[name], actual[name], evidence_map, target, diagnostics
        )
        normalized[name] = (lines, tokens)
        skeletons[name] = _skeleton_body(
            body,
            _fixed_block_contents(plan_skills[name].get("fixed_blocks", [])),
            entities,
        )

    names = sorted(normalized)
    for index, left_name in enumerate(names):
        for right_name in names[index + 1 :]:
            line_score = _line_similarity(normalized[left_name][0], normalized[right_name][0])
            token_score = _token_similarity(normalized[left_name][1], normalized[right_name][1])
            if line_score >= LINE_FAIL_THRESHOLD or token_score >= TOKEN_FAIL_THRESHOLD:
                repeated = sorted(set(normalized[left_name][0]) & set(normalized[right_name][0]))[:3]
                detail = "; repeated: " + " | ".join(repeated) if repeated else ""
                _diag(
                    diagnostics,
                    "SKILL_SIMILARITY",
                    f"{left_name} and {right_name} duplicate substantive content "
                    f"(line={line_score:.3f}, token={token_score:.3f}){detail}",
                )
            elif token_score >= TOKEN_WARN_THRESHOLD:
                _diag(
                    diagnostics,
                    "SKILL_SIMILARITY_WARN",
                    f"{left_name} and {right_name} have high token similarity "
                    f"(line={line_score:.3f}, token={token_score:.3f})",
                    "warning",
                )
            _compare_skeletons(
                left_name,
                skeletons[left_name],
                right_name,
                skeletons[right_name],
                diagnostics,
            )

    block_owners: defaultdict[tuple[str, ...], set[str]] = defaultdict(set)
    for name, (lines, _) in normalized.items():
        for size in range(3, min(7, len(lines) + 1)):
            for index in range(len(lines) - size + 1):
                block = tuple(lines[index : index + size])
                if sum(len(_tokens(line)) for line in block) >= 20:
                    block_owners[block].add(name)
    for block, owners in sorted(block_owners.items()):
        if len(owners) > 1 and len(block) == max(
            len(candidate)
            for candidate, candidate_owners in block_owners.items()
            if candidate_owners == owners
        ):
            _diag(
                diagnostics,
                "REPEATED_BLOCK",
                f"repeated substantive block across {', '.join(sorted(owners))}: {' | '.join(block[:2])}",
            )

    _report_skeleton_blocks(skeletons, diagnostics)


def _report_skeleton_blocks(
    skeletons: dict[str, list[str]], diagnostics: list[Diagnostic]
) -> None:
    """Adjacent skeleton lines shared by two skills are a copied passage.

    `REPEATED_BLOCK` needs three consecutive byte-identical lines, which one
    substituted noun anywhere in the run defeats. On the skeleton the run
    survives substitution, so the block shrinks to `SKELETON_BLOCK_SIZE`
    lines; the token floor is kept so two short scaffolding sentences are not
    enough on their own. Only the longest block per owner set is reported, so
    a copied passage produces one diagnostic instead of one per sub-window.

    Deliberately a warning, unlike `REPEATED_BLOCK`. Measured on 2371 pairs of
    honest hand-written skills this fires five times - on a shared task-counter
    instruction and on the shared confidence rubric of a parallel scanner
    family - all of them legitimate reuse of one policy sentence rather than a
    duplicated skill. Blocking on that would fail honest generations, so the
    error verdict stays with the two calibrated similarity metrics and this
    pass only names the passage that is worth a human look.
    """
    block_owners: defaultdict[tuple[str, ...], set[str]] = defaultdict(set)
    for name, lines in skeletons.items():
        for size in range(SKELETON_BLOCK_SIZE, min(7, len(lines) + 1)):
            for index in range(len(lines) - size + 1):
                block = tuple(lines[index : index + size])
                if sum(len(_tokens(line)) for line in block) >= 20:
                    block_owners[block].add(name)
    for block, owners in sorted(block_owners.items()):
        if len(owners) > 1 and len(block) == max(
            len(candidate)
            for candidate, candidate_owners in block_owners.items()
            if candidate_owners == owners
        ):
            _diag(
                diagnostics,
                "SKILL_TEMPLATE_BLOCK",
                f"repeated prose skeleton across {', '.join(sorted(owners))}: "
                f"{' | '.join(block[:2])}",
                "warning",
            )


def validate(
    skills_dir: Path,
    plan_path: Path,
    target: Path,
    registry_path: Path = DEFAULT_REGISTRY,
    *,
    allow_partial_skills: bool = False,
) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    target = target.expanduser().resolve()
    skills_dir = skills_dir.expanduser()
    plan_path = plan_path.expanduser().resolve()
    if not target.is_dir():
        _diag(diagnostics, "TARGET_INVALID", f"target is not a directory: {target}")
        return sorted(set(diagnostics))
    if not skills_dir.is_dir():
        _diag(diagnostics, "SKILLS_DIR_INVALID", f"skills directory is not a directory: {skills_dir}")
        return sorted(set(diagnostics))
    if skills_dir.is_symlink():
        _diag(diagnostics, "SKILLS_DIR_UNSAFE", f"skills directory must not be a symlink: {skills_dir}")
        return sorted(set(diagnostics))
    skills_dir = skills_dir.resolve()

    plan = _load_plan(plan_path, diagnostics)
    plan_skills, evidence_map = (
        _validate_plan(plan, plan_path, target, registry_path, diagnostics)
        if plan
        else ({}, {})
    )
    if plan.get("schema_version") in {"1.0", "1.1"}:
        _diag(
            diagnostics,
            "LEGACY_PLAN_PUBLICATION_INELIGIBLE",
            f"schema {plan['schema_version']} is audit-only and cannot validate "
            "authored or partially authored generation output; migrate to schema 1.2",
        )
    _validate_authored_inventory(
        skills_dir,
        plan_skills,
        evidence_map,
        target,
        diagnostics,
        allow_partial_skills=allow_partial_skills,
    )
    return sorted(set(diagnostics))


def validate_plan(
    plan_path: Path,
    target: Path,
    registry_path: Path = DEFAULT_REGISTRY,
) -> list[Diagnostic]:
    """Validate evidence and per-skill contracts before authoring starts."""
    diagnostics: list[Diagnostic] = []
    plan_path = plan_path.expanduser().resolve()
    target = target.expanduser().resolve()
    if not target.is_dir():
        _diag(diagnostics, "TARGET_INVALID", f"target is not a directory: {target}")
        return sorted(set(diagnostics))
    plan = _load_plan(plan_path, diagnostics)
    if plan:
        _validate_plan(plan, plan_path, target, registry_path, diagnostics)
    return sorted(set(diagnostics))


def validate_agent_routing(
    agents_dir: Path,
    plan_path: Path,
    target: Path,
    *,
    require_invokes: bool = False,
    registry_path: Path = DEFAULT_REGISTRY,
    validate_plan_first: bool = True,
) -> list[Diagnostic]:
    """Validate generated wrapper selection against skill routing contracts."""
    diagnostics = (
        validate_plan(plan_path, target, registry_path)
        if validate_plan_first
        else []
    )
    if any(item.severity == "error" for item in diagnostics):
        return diagnostics
    plan = _load_plan(plan_path.expanduser().resolve(), diagnostics)
    contracts = {
        item["name"]: item
        for item in _as_list(plan.get("skills"))
        if isinstance(item, dict) and _is_nonempty_string(item.get("name"))
    }
    agents_dir = agents_dir.expanduser().resolve()
    if not agents_dir.is_dir():
        _diag(
            diagnostics,
            "AGENTS_DIR_INVALID",
            f"agents directory is not a directory: {agents_dir}",
        )
        return sorted(set(diagnostics))
    actual_agents = {
        path.name.removesuffix("-agent.md")
        for path in agents_dir.glob("*-agent.md")
        if path.is_file() and not path.is_symlink()
    }
    for path in sorted(agents_dir.glob("*.md")):
        if path.name == "README.md":
            continue
        if not path.name.endswith("-agent.md") or path.is_symlink():
            _diag(
                diagnostics,
                "AGENT_ROUTING_NONCONFORMING_FILE",
                f"agent roster contains a nonconforming file: {path.name}",
            )
    for unplanned in sorted(actual_agents - set(contracts)):
        _diag(
            diagnostics,
            "AGENT_ROUTING_UNPLANNED",
            f"{unplanned}: wrapper has no skill contract",
        )
    for name, contract in sorted(contracts.items()):
        path = agents_dir / f"{name}-agent.md"
        if not path.is_file() or path.is_symlink():
            _diag(
                diagnostics,
                "AGENT_ROUTING_MISSING",
                f"{name}: matching agent wrapper is missing",
            )
            continue
        text = path.read_text(encoding="utf-8")
        frontmatter, body = _parse_frontmatter(text)
        if frontmatter.get("name") != f"{name}-agent":
            _diag(
                diagnostics,
                "AGENT_FRONTMATTER_NAME",
                f"{name}: wrapper name must be {name}-agent",
            )
        invokes = frontmatter.get("invokes")
        if (require_invokes and invokes != name) or (
            invokes is not None and invokes != name
        ):
            _diag(
                diagnostics,
                "AGENT_INVOKES_MISMATCH",
                f"{name}: wrapper must invoke exactly {name}",
            )
        description = frontmatter.get("description", "")
        triggers = contract.get("triggers")
        if not isinstance(triggers, dict):
            triggers = {}
        if not _contract_matches(
            triggers.get("positive"),
            description + "\n" + body,
        ):
            _diag(
                diagnostics,
                "AGENT_POSITIVE_ROUTING",
                f"{name}: positive trigger is not traceable to wrapper",
            )
        if not _contract_matches(triggers.get("negative"), body):
            _diag(
                diagnostics,
                "AGENT_NEGATIVE_ROUTING",
                f"{name}: negative deferral is not traceable to wrapper",
            )
        if not _contract_matches(contract.get("output_contract"), body):
            _diag(
                diagnostics,
                "AGENT_OUTPUT_ROUTING",
                f"{name}: expected result is not traceable to wrapper",
            )
        for sibling in _as_list(contract.get("nearest_siblings")):
            if isinstance(sibling, dict) and (
                str(sibling.get("name", "")).lower() not in body.lower()
                or not _contract_matches(sibling.get("boundary"), body)
            ):
                _diag(
                    diagnostics,
                    "AGENT_SIBLING_ROUTING",
                    f"{name}: sibling deferral/boundary is not traceable",
                )
        writes_expected = bool(contract.get("writes"))
        writes_actual = frontmatter.get("writes", "").strip().lower() == "true"
        if writes_actual != writes_expected:
            _diag(
                diagnostics,
                "AGENT_WRITES_MISMATCH",
                f"{name}: wrapper writes flag does not match contract",
            )
        if re.search(
            rf"\b(use|select)\b.*\b{_name_pattern(name)}\b.*"
            rf"\b(governed|handled|skill)\b",
            description,
            re.I,
        ):
            _diag(
                diagnostics,
                "AGENT_CIRCULAR_ROUTING",
                f"{name}: wrapper description is circular",
            )
    return sorted(set(diagnostics))


def validate_flow_routing(
    commands_dir: Path,
    plan_path: Path,
    target: Path,
    registry_path: Path = DEFAULT_REGISTRY,
) -> list[Diagnostic]:
    """Validate that generated flows route specialists conditionally."""
    diagnostics = validate_plan(plan_path, target, registry_path)
    if any(item.severity == "error" for item in diagnostics):
        return diagnostics
    plan = _load_plan(plan_path.expanduser().resolve(), diagnostics)
    specialists = {
        item["name"]: item
        for item in plan.get("skills", [])
        if str(item.get("category", "")).lower() in SPECIALIST_CATEGORIES
    }
    commands_dir = commands_dir.expanduser().resolve()
    if not commands_dir.is_dir():
        _diag(
            diagnostics,
            "COMMANDS_DIR_INVALID",
            f"commands directory is not a directory: {commands_dir}",
        )
        return sorted(set(diagnostics))
    for flow_name in ("flow-feature", "flow-review"):
        path = commands_dir / f"{flow_name}.md"
        if not path.is_file() or path.is_symlink():
            _diag(
                diagnostics,
                "FLOW_ROUTING_MISSING",
                f"{flow_name}: generated flow command is missing",
            )
            continue
        text = path.read_text(encoding="utf-8")
        _, body = _parse_frontmatter(text)
        if specialists and "specialist routing" not in body.lower():
            _diag(
                diagnostics,
                "FLOW_SPECIALIST_SECTION",
                f"{flow_name}: Specialist Routing section is missing",
            )
        frontmatter_text = text.split("---", 2)[1] if text.startswith("---") else ""
        for name, contract in sorted(specialists.items()):
            agent_name = f"{name}-agent"
            if agent_name in frontmatter_text:
                _diag(
                    diagnostics,
                    "FLOW_SPECIALIST_UNCONDITIONAL",
                    f"{flow_name}: specialist {agent_name} is in a static stage",
                )
            matching_lines = [
                line
                for line in body.splitlines()
                if agent_name.lower() in line.lower()
            ]
            routing_text = "\n".join(matching_lines)
            if not matching_lines:
                _diag(
                    diagnostics,
                    "FLOW_SPECIALIST_UNROUTED",
                    f"{flow_name}: specialist {agent_name} is absent from routing",
                )
            if not _contract_matches(
                contract.get("triggers", {}).get("positive"), routing_text
            ) or not _contract_matches(
                contract.get("triggers", {}).get("negative"), routing_text
            ):
                _diag(
                    diagnostics,
                    "FLOW_SPECIALIST_TRIGGER",
                    f"{flow_name}: {agent_name} lacks positive/negative scope routing",
                )
            if flow_name == "flow-review" and contract.get("writes"):
                pattern = rf"{re.escape(agent_name)}[^\n]*(skip|not selected|write-capable)"
                if not re.search(pattern, routing_text, re.I):
                    _diag(
                        diagnostics,
                        "FLOW_REVIEW_WRITER",
                        f"{flow_name}: write-capable {agent_name} must be explicitly skipped",
                    )
    return sorted(set(diagnostics))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skills-dir")
    parser.add_argument("--agents-dir")
    parser.add_argument("--plan", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument(
        "--registry",
        default=str(DEFAULT_REGISTRY),
        help="machine-readable candidate registry",
    )
    validation_mode = parser.add_mutually_exclusive_group()
    validation_mode.add_argument(
        "--plan-only",
        action="store_true",
        help="validate evidence and contracts before any SKILL.md files exist",
    )
    validation_mode.add_argument(
        "--allow-partial-skills",
        action="store_true",
        help="validate all plan contracts and only authored SKILL.md files that exist",
    )
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    if args.plan_only:
        diagnostics = validate_plan(
            Path(args.plan), Path(args.target), Path(args.registry)
        )
    elif not args.skills_dir:
        parser.error("--skills-dir is required unless --plan-only is used")
    else:
        diagnostics = validate(
            Path(args.skills_dir),
            Path(args.plan),
            Path(args.target),
            Path(args.registry),
            allow_partial_skills=args.allow_partial_skills,
        )
        if args.agents_dir:
            diagnostics = sorted(
                set(
                    diagnostics
                    + validate_agent_routing(
                        Path(args.agents_dir),
                        Path(args.plan),
                        Path(args.target),
                        registry_path=Path(args.registry),
                        validate_plan_first=False,
                    )
                )
            )
    errors = [item for item in diagnostics if item.severity == "error"]
    if args.as_json:
        print(
            json.dumps(
                {
                    "valid": not errors,
                    "error_count": len(errors),
                    "warning_count": len(diagnostics) - len(errors),
                    "diagnostics": [item.as_dict() for item in diagnostics],
                },
                indent=2,
                sort_keys=True,
            )
        )
    else:
        for item in diagnostics:
            stream = sys.stderr if item.severity == "error" else sys.stdout
            print(f"{item.severity.upper()} [{item.code}] {item.message}", file=stream)
        print(
            f"skill quality: {'PASS' if not errors else 'FAIL'} "
            f"({len(errors)} errors, {len(diagnostics) - len(errors)} warnings)"
        )
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
