"""Stack-neutral service discovery and planning; never execute service code."""
from __future__ import annotations

from collections import deque
from datetime import date, datetime, timezone
from functools import lru_cache
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess

import portable_fs as fs

MAX_BYTES = 2 * 1024 * 1024
MAX_SOURCE_BYTES = 256 * 1024
MAX_SERVICES = 500
MAX_ITEMS = 100
MAX_CONTEXT = 64000
IDENTIFIER = re.compile(r"[a-z][a-z0-9._-]{0,79}\Z")
DIGEST = re.compile(r"[0-9a-f]{64}\Z")
KINDS = {"policy", "spec", "contract", "code", "test", "memory"}
SECRET = re.compile(
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----|\bAKIA[A-Z0-9]{16}\b|"
    r"\bsk-[A-Za-z0-9_-]{20,}|"
    r"(?i:\b(?:password|api[_-]?key|access[_-]?token)\s*[:=]\s*['\"]?"
    r"[A-Za-z0-9_+/=-]{16,})"
)
BLOCKED = {".git", ".ssh", ".aws", ".kube", "node_modules", "vendor",
           ".venv", "__pycache__", "secrets", ".secrets", "credentials"}


class SystemError(Exception):
    """Invalid input or refused filesystem access."""


def encoded(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                      allow_nan=False)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def text(value, label, limit=2000):
    if (not isinstance(value, str) or not value.strip() or len(value) > limit
            or any(ord(c) < 32 or ord(c) == 127 for c in value)
            or SECRET.search(value)):
        raise SystemError("Invalid or sensitive " + label)
    return value


def identifier(value):
    if not isinstance(value, str) or not IDENTIFIER.fullmatch(value):
        raise SystemError("Invalid identifier")
    return value


def fields(value, required, optional=()):
    if not isinstance(value, dict) or set(value) - set(required) - set(optional) or set(required) - set(value):
        raise SystemError("Invalid object fields (missing or unknown keys)")


def items(value, limit=MAX_ITEMS):
    if not isinstance(value, list) or len(value) > limit:
        raise SystemError("Invalid or oversized list")
    return value


def choice(value, allowed, label):
    if not isinstance(value, str) or value not in allowed:
        raise SystemError("Invalid " + label)


def relative(value):
    text(value, "source path", 1024)
    path = PurePosixPath(value)
    if (path.is_absolute() or path.as_posix() != value or "\\" in value or any(p in {".", ".."} for p in value.split("/"))
            or any(p.casefold() in BLOCKED or p.casefold().startswith(".env") for p in path.parts)
            or any(part in {"memory-bank", "project-brain"} and next_part in {"local", "archive", "dynamic", "control"}
                   for part, next_part in zip(path.parts, path.parts[1:]))
            or path.suffix.casefold() in {".pem", ".key", ".p12", ".pfx", ".sqlite", ".db"}
            or path.name.casefold() in {"credentials", "credentials.json", "id_rsa", "id_ed25519"}):
        raise SystemError("Refused source path")
    return value


def absolute(path):
    return Path(os.path.abspath(str(path)))


def open_directory(path):
    """Open every component without following links, including ancestors.

    Windows opens each component from the drive root through a rooted handle and
    refuses junctions and other reparse points.
    """
    return fs.open_target_directory(absolute(path))


def read_file(root, name, limit=MAX_BYTES):
    name = relative(name)
    fd = open_directory(root)
    try:
        parts = PurePosixPath(name).parts
        for part in parts[:-1]:
            child = fs.open(part, os.O_RDONLY | fs.O_DIRECTORY | fs.O_NOFOLLOW, dir_fd=fd)
            fs.close(fd)
            fd = child
        file_fd = fs.open(parts[-1], os.O_RDONLY | fs.O_NOFOLLOW | fs.O_NONBLOCK, dir_fd=fd)
        try:
            info = fs.fstat(file_fd)
            if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > limit:
                raise SystemError("Source must be a bounded regular file without hard links")
            with os.fdopen(file_fd, "rb", closefd=False) as handle:
                raw = handle.read(limit + 1)
            if len(raw) > limit:
                raise SystemError("Source exceeds size limit")
            return raw
        finally:
            fs.close(file_fd)
    except ValueError:
        # Windows refuses device names, streams and trailing dots as components.
        raise SystemError("Refused source path") from None
    finally:
        fs.close(fd)


def parse_json(raw):
    def pairs(values):
        result = {}
        for key, value in values:
            if key in result:
                raise SystemError("Duplicate JSON key")
            result[key] = value
        return result

    def invalid_constant(_):
        raise SystemError("Non-finite JSON number")

    def number(value):
        parsed = float(value)
        if not math.isfinite(parsed):
            raise SystemError("Non-finite JSON number")
        return parsed

    def integer(value):
        if len(value) > 20:
            raise SystemError("Oversized JSON integer")
        return int(value)

    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=pairs,
                          parse_constant=invalid_constant, parse_float=number, parse_int=integer)
    except (UnicodeError, ValueError, RecursionError) as error:
        raise SystemError("Invalid UTF-8 JSON") from error


def version(value):
    if type(value) is not int or value != 1:
        raise SystemError("Unsupported schema_version")


def source(value):
    fields(value, ("path", "kind"))
    relative(value["path"])
    choice(value["kind"], KINDS, "source kind")
    if is_memory_path(value["path"]) and value["kind"] != "memory":
        raise SystemError("Memory chunks must use the memory source kind")


def is_memory_path(path):
    parts = PurePosixPath(path).parts
    return "memory-bank" in parts and "chunks" in parts


def manifest(value, service_id):
    fields(value, ("schema_version", "id", "description", "owner", "capabilities", "provides", "consumes", "sources"),
           ("relationships_complete",))
    version(value["schema_version"])
    if identifier(value["id"]) != service_id:
        raise SystemError("Manifest service ID does not match registry")
    text(value["description"], "description")
    text(value["owner"], "owner", 200)
    if type(value.get("relationships_complete", False)) is not bool:
        raise SystemError("relationships_complete must be boolean")
    for key in ("capabilities", "provides", "consumes", "sources"):
        items(value[key])
    seen = set()
    for capability in value["capabilities"]:
        fields(capability, ("id", "description", "status", "sources"), ("keywords",))
        key = identifier(capability["id"])
        if key in seen:
            raise SystemError("Duplicate capability")
        seen.add(key)
        text(capability["description"], "capability description")
        choice(capability["status"], {"implemented", "partial", "planned", "unknown"}, "capability status")
        for path in items(capability["sources"]):
            relative(path)
        for keyword in items(capability.get("keywords", [])):
            text(keyword, "keyword", 200)
    seen = set()
    for contract in value["provides"]:
        fields(contract, ("id", "kind", "version", "sources"))
        key = identifier(contract["id"])
        if key in seen:
            raise SystemError("Duplicate provided contract")
        seen.add(key)
        choice(contract["kind"], {"http", "event", "rpc", "graphql", "other"}, "contract kind")
        text(contract["version"], "contract version", 100)
        for path in items(contract["sources"]):
            relative(path)
    seen = set()
    for contract in value["consumes"]:
        fields(contract, ("service", "contract", "version"))
        key = (identifier(contract["service"]), identifier(contract["contract"]))
        if key in seen:
            raise SystemError("Duplicate consumed contract")
        seen.add(key)
        text(contract["version"], "contract version", 100)
    seen = set()
    for entry in value["sources"]:
        source(entry)
        if entry["path"] in seen:
            raise SystemError("Duplicate source")
        seen.add(entry["path"])
    all_sources = {s["path"] for s in value["sources"]}
    for entry in [*value["capabilities"], *value["provides"]]:
        all_sources.update(entry["sources"])
    if len(all_sources) > MAX_ITEMS:
        raise SystemError("Too many distinct sources in service manifest")
    return value


def inside(path, root):
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def git_head(root):
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
    try:
        result = subprocess.run(["git", "-C", str(root), "rev-parse", "--verify", "HEAD"],
                                capture_output=True, text=True, timeout=5, env=env)
        head = result.stdout.strip()
        return head if result.returncode == 0 and re.fullmatch(r"[0-9a-f]{40,64}", head) else None
    except (OSError, subprocess.TimeoutExpired):
        return None


@lru_cache(maxsize=1)
def memory_contract():
    """Load the trusted checkout's shared Python contract, never target code."""
    path = Path(__file__).resolve().parent.parent / "PHP Core/memory-bank/scripts/validate.py"
    spec = importlib.util.spec_from_file_location("ai_system_native_memory_contract", path)
    if spec is None or spec.loader is None:
        raise SystemError("The source checkout's native memory contract is unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class System:
    def __init__(self, config, allow_roots=()):
        self.config = absolute(config)
        self.root = self.config.parent
        self.allow_roots = [self.root] + [absolute(path) for path in allow_roots]
        self.snapshot = {}
        self.missing = set()
        self.warnings = []
        self.warning_keys = set()
        self.config_data = parse_json(self.read("__system__", self.root, self.config.name))
        fields(self.config_data, ("schema_version", "name", "services", "shared_sources"))
        version(self.config_data["schema_version"])
        text(self.config_data["name"], "system name", 200)
        items(self.config_data["services"], MAX_SERVICES)
        for entry in items(self.config_data["shared_sources"]):
            source(entry)
        self.services = {}
        for entry in self.config_data["services"]:
            fields(entry, ("id", "root", "manifest"))
            sid = identifier(entry["id"])
            if sid in self.services:
                raise SystemError("Duplicate service ID")
            location = text(entry["root"], "service root", 4096)
            relative(entry["manifest"])
            self.services[sid] = {"id": sid, "root": absolute(self.root / location),
                                  "manifest_path": entry["manifest"], "manifest": None,
                                  "access": "unavailable", "head": None}
        for sid, service in sorted(self.services.items()):
            if not any(inside(service["root"], root) for root in self.allow_roots):
                service["access"] = "denied"
                self.warn(sid, "root_not_allowed")
                continue
            try:
                raw = self.read(sid, service["root"], service["manifest_path"])
            except (OSError, SystemError):
                self.warn(sid, "manifest_unavailable")
                continue
            service["manifest"] = manifest(parse_json(raw), sid)
            service["access"] = "available"
            service["head"] = git_head(service["root"])
            if not service["manifest"].get("relationships_complete", False):
                self.warn(sid, "relationships_incomplete")
        self.edges = []
        for sid, service in sorted(self.services.items()):
            if service["manifest"] is None:
                continue
            for consume in service["manifest"]["consumes"]:
                provider = self.services.get(consume["service"])
                if provider is None:
                    raise SystemError("Contract references an unregistered service")
                provided = next((c for c in (provider["manifest"] or {}).get("provides", [])
                                 if c["id"] == consume["contract"]), None)
                if provider["manifest"] is not None and provided is None:
                    raise SystemError("Consumer references a contract the provider does not declare")
                if provided is None:
                    self.warn(sid, "provider_contract_unavailable")
                elif provided["version"] != consume["version"]:
                    self.warn(sid, "contract_version_mismatch")
                self.edges.append({"provider": consume["service"], "consumer": sid,
                                   "contract": consume["contract"], "version": consume["version"],
                                   "provenance": "declared"})

    def warn(self, service, reason, path=None):
        entry = {"service": service, "reason": reason}
        if path is not None:
            entry["path"] = path
        key = (service, reason, path)
        if key not in self.warning_keys:
            self.warning_keys.add(key)
            self.warnings.append(entry)

    def read(self, sid, root, path):
        try:
            raw = read_file(root, path, MAX_BYTES if sid == "__system__" and path == self.config.name else MAX_SOURCE_BYTES)
        except FileNotFoundError:
            self.missing.add((sid, path))
            raise
        self.snapshot[(sid, path)] = digest(raw)
        return raw

    def catalog(self):
        return {"schema_version": 1, "system": self.config_data["name"],
                "services": [{"id": sid, "root": str(service["root"]),
                              "access": service["access"], "head": service["head"],
                              "declared": service["manifest"]}
                             for sid, service in sorted(self.services.items())],
                "relationships": self.edges, "warnings": self.warnings}

    def diagram(self):
        """Mermaid uses only validated IDs, never raw descriptions or source text."""
        names = {sid: "s" + str(index) for index, sid in enumerate(sorted(self.services))}
        lines = ["flowchart LR"]
        for sid, node in names.items():
            availability = "" if self.services[sid]["access"] == "available" else " (unavailable)"
            lines.append('    ' + node + '["' + sid + availability + '"]')
        for edge in self.edges:
            lines.append("    " + names[edge["provider"]] + " -->|" + edge["contract"] + "| " + names[edge["consumer"]])
        return "\n".join(lines) + "\n"

    def locate(self, query):
        text(query, "task", 2000)
        words = set(re.findall(r"[^\W_]{3,}", query.casefold()))
        candidates = []
        for sid, service in sorted(self.services.items()):
            data = service["manifest"]
            if data is None:
                continue
            matches = []
            for capability in data["capabilities"]:
                subject = " ".join([capability["id"], capability["description"],
                                    *capability.get("keywords", [])]).casefold()
                matched = sorted(words & set(re.findall(r"[^\W_]{3,}", subject)))
                if matched:
                    matches.append({"capability": capability["id"], "terms": matched,
                                    "declared_status": capability["status"]})
            if matches:
                candidates.append({"service": sid, "matches": matches, "provenance": "inferred"})
        return {"candidates": candidates,
                "selection": "suggested" if len(candidates) == 1 else "needs_selection"}

    def impact(self, seeds, contracts=(), depth=32):
        if type(depth) is not int or not 0 <= depth <= 64:
            raise SystemError("impact depth must be between 0 and 64")
        selected = {sid: {"service": sid, "reason": "selected", "path": [sid]} for sid in sorted(set(seeds))}
        for sid in selected:
            if sid not in self.services:
                raise SystemError("Selected service is not registered")
        changed = set()
        for value in contracts:
            if not isinstance(value, str) or value.count(":") != 1:
                raise SystemError("Contract selector must be SERVICE:CONTRACT")
            sid, cid = value.split(":")
            identifier(sid)
            identifier(cid)
            data = self.services.get(sid, {}).get("manifest")
            if data is None or not any(c["id"] == cid for c in data["provides"]):
                raise SystemError("Selected contract is unavailable or undeclared")
            changed.add((sid, cid))
            selected.setdefault(sid, {"service": sid, "reason": "contract_provider", "path": [sid]})
        queue = deque(sorted(selected))
        traversed = set()
        while queue:
            sid = queue.popleft()
            path = selected[sid]["path"]
            for edge in self.edges:
                if edge["provider"] != sid:
                    continue
                if changed and len(path) == 1 and (sid, edge["contract"]) not in changed:
                    continue
                consumer = edge["consumer"]
                traversed.add((sid, consumer))
                if consumer in path:
                    self.warn(consumer, "dependency_cycle")
                if consumer in selected:
                    continue
                if len(path) - 1 >= depth:
                    self.warn(consumer, "impact_depth_exceeded")
                    continue
                selected[consumer] = {"service": consumer, "reason": "potential_consumer",
                                      "contract": edge["contract"], "path": path + [consumer]}
                queue.append(consumer)
        # Multiple explicit origins can hide a cycle from shortest-path checks:
        # both ends are already selected before either traversal reaches them.
        indegree = {sid: 0 for sid in selected}
        outgoing = {sid: set() for sid in selected}
        for provider, consumer in traversed:
            if provider in selected and consumer in selected:
                outgoing[provider].add(consumer)
                indegree[consumer] += 1
        ready = deque(sorted(sid for sid, count in indegree.items() if count == 0))
        removed = 0
        while ready:
            sid = ready.popleft()
            removed += 1
            for consumer in sorted(outgoing[sid]):
                indegree[consumer] -= 1
                if indegree[consumer] == 0:
                    ready.append(consumer)
        if removed != len(selected):
            self.warn("__system__", "dependency_cycle")
        return [selected[sid] for sid in sorted(selected)]

    def source_content(self, sid, root, entry):
        path = entry["path"]
        try:
            raw = self.read(sid, root, path)
            content = raw.decode("utf-8")
            if "\x00" in content or SECRET.search(content):
                raise SystemError("sensitive_or_binary")
            if entry["kind"] == "memory":
                if not content.startswith("---\n") or "\n---\n" not in content[4:]:
                    raise SystemError("unverified_memory")
                meta = parse_json(content[4:content.index("\n---\n", 4)].encode("utf-8"))
                if (not isinstance(meta, dict) or meta.get("status") != "active"
                        or "auto-promoted" in items(meta.get("tags", []))
                        or meta.get("superseded_by") is not None):
                    raise SystemError("ineligible_memory")
                today = datetime.now(timezone.utc).date()
                boundaries = [date.fromisoformat(meta["review_after"])]
                if meta.get("valid_to") is not None:
                    boundaries.append(date.fromisoformat(meta["valid_to"]))
                if min(boundaries) < today:
                    raise SystemError("memory_review_due")
                # Honor a consuming project's more restrictive native privacy policy.
                # Absence is allowed; an existing unreadable/malformed config is not.
                native_path = "project-brain/config/runtime.json"
                try:
                    native_raw = self.read(sid, root, native_path)
                except FileNotFoundError:
                    native = {}
                else:
                    native = parse_json(native_raw)
                    if not isinstance(native, dict):
                        raise SystemError("ineligible_memory")
                allowed = items(native.get("allowed_privacy", ["public", "team"]))
                authority = items(native.get("allowed_authority", ["observed", "verified"]))
                # Native durable documents are public/verified; private Brain
                # records are deliberately never read by this adapter.
                if "public" not in allowed or "verified" not in authority:
                    raise SystemError("ineligible_memory")
                cited = {relative(p.split("#", 1)[0]) for p in items(meta["sources"])}
                fingerprints = items(meta["source_digests"])
                if not cited or len(fingerprints) != len(cited):
                    raise SystemError("unverified_memory")
                seen = set()
                for fingerprint in fingerprints:
                    fields(fingerprint, ("path", "sha256"))
                    name = relative(fingerprint["path"])
                    if name not in cited or name in seen or not isinstance(fingerprint["sha256"], str):
                        raise SystemError("unverified_memory")
                    seen.add(name)
                    cited_raw = self.read(sid, root, name)
                    if digest(cited_raw) != fingerprint["sha256"]:
                        raise SystemError("stale_memory")
                contract = memory_contract()
                try:
                    contract.validate_metadata(Path(root) / path, meta, Path(root))
                except contract.ValidationError as error:
                    raise SystemError("invalid_native_memory") from error
                if any(pattern.search(content) for pattern in contract.SECRET_PATTERNS.values()):
                    raise SystemError("sensitive_memory")
            return {"service": sid, "path": path, "kind": entry["kind"],
                    "sha256": digest(raw), "trust": "source_evidence",
                    "excerpt": content[:2000], "truncated": len(content) > 2000}
        except (OSError, UnicodeError, ValueError, KeyError, TypeError, SystemError):
            self.warn(sid, "source_unavailable_or_ineligible", path)
            return None

    def plan(self, query, change_id, services=(), contracts=(), budget=8000, depth=32):
        text(query, "task", 2000)
        identifier(change_id)
        if type(budget) is not int or not 128 <= budget <= MAX_CONTEXT:
            raise SystemError("Context budget must be 128..64000 characters")
        routing = self.locate(query)
        seeds = list(services)
        inferred = False
        if not seeds and not contracts and len(routing["candidates"]) == 1:
            seeds = [routing["candidates"][0]["service"]]
            inferred = True
        selected = self.impact(seeds, contracts, depth)
        capsule = {"task": query, "services": [item["service"] for item in selected], "sources": []}
        if len(encoded(capsule)) > budget:
            raise SystemError("Context budget cannot hold task and service identities")
        omissions = []
        sources = [("__system__", self.root, entry) for entry in self.config_data["shared_sources"]] if selected else []
        for item in selected:
            sid = item["service"]
            service = self.services[sid]
            data = service["manifest"]
            if data is None:
                continue
            declared = {entry["path"]: entry for entry in data["sources"]}
            for capability in data["capabilities"]:
                for path in capability["sources"]:
                    declared.setdefault(path, {"path": path, "kind": "memory" if is_memory_path(path) else "code"})
            for contract in data["provides"]:
                for path in contract["sources"]:
                    declared.setdefault(path, {"path": path, "kind": "memory" if is_memory_path(path) else "contract"})
            # Round-robin below keeps a large first service from owning the budget.
            sources.extend((sid, service["root"], entry) for entry in declared.values())
        groups = {}
        priority = {"policy": 0, "contract": 1, "spec": 2, "memory": 3, "code": 4, "test": 5}
        for sid, root, entry in sources:
            groups.setdefault(sid, []).append((root, entry))
        for group in groups.values():
            group.sort(key=lambda pair: (priority[pair[1]["kind"]], pair[1]["path"]))
        queue = deque(sorted(groups))
        while queue:
            sid = queue.popleft()
            root, entry = groups[sid].pop(0)
            if groups[sid]:
                queue.append(sid)
            content = self.source_content(sid, root, entry)
            if content is None:
                omissions.append({"service": sid, "path": entry["path"], "reason": "ineligible"})
                continue
            trial = {**capsule, "sources": capsule["sources"] + [content]}
            if len(encoded(trial)) <= budget:
                capsule = trial
            else:
                omissions.append({"service": sid, "path": entry["path"], "reason": "budget"})
        steps = [{"id": "scope", "depends_on": [], "goal": "Confirm affected services and missing relationships"},
                 {"id": "contracts", "depends_on": ["scope"], "goal": "Agree invariants, changed contracts, compatibility and delivery order"}]
        previous = "contracts"
        # This is an investigation order, not a deployment order. Contract
        # agreement must establish the actual implementation/release sequencing.
        for item in sorted(selected, key=lambda item: (len(item["path"]), item["service"])):
            sid = item["service"]
            step_id = "service-" + sid
            steps.append({"id": step_id, "service": sid, "depends_on": [previous],
                          "task_reference": {"external_id": change_id + "/" + sid, "created": False},
                          "goal": "Inspect local policy and sources, prepare scoped changes and local validation",
                          "acceptance": ["Follow the service's policy and source contracts",
                                         "Report local checks run and checks not run",
                                         "Do not infer deployment order from this investigation order"]})
            previous = step_id
        steps.extend([
            {"id": "verify", "depends_on": [previous],
             "goal": "Check local results, producer/consumer compatibility and the end-to-end scenario"},
            {"id": "knowledge", "depends_on": ["verify"],
             "goal": "Update source catalogs and reusable local/system knowledge through their owning runtime"}])
        state = "needs_selection" if not selected else "needs_review"
        return {"schema_version": 1, "kind": "ai-system-plan", "change_id": change_id,
                "system": self.config_data["name"], "status": state, "executed": False,
                "routing": {**routing, "selection_provenance": "inferred" if inferred else "explicit"},
                "impact": selected, "relationships": [edge for edge in self.edges
                    if edge["provider"] in capsule["services"] and edge["consumer"] in capsule["services"]],
                "context": capsule, "context_chars": len(encoded(capsule)), "context_budget_chars": budget,
                "memory_layout": {"system": {"brain": "project-brain", "bank": "memory-bank"},
                                  "services": [{"service": item["service"], "brain": "project-brain",
                                                "bank": "memory-bank", "tasks_created": False}
                                               for item in selected]},
                "omitted_sources": omissions, "warnings": self.warnings, "steps": steps,
                "snapshot": {"files": [{"service": sid, "path": path, "sha256": sha}
                                      for (sid, path), sha in sorted(self.snapshot.items())],
                             "missing": [{"service": sid, "path": path} for sid, path in sorted(self.missing)],
                             "heads": {sid: self.services[sid]["head"] for sid in capsule["services"]}}}

    def verify(self, plan):
        if (not isinstance(plan, dict) or plan.get("kind") != "ai-system-plan"
                or type(plan.get("schema_version")) is not int or plan["schema_version"] != 1):
            raise SystemError("Not an ai-system plan")
        snapshot = plan.get("snapshot")
        fields(snapshot, ("files", "missing", "heads"))
        if not isinstance(snapshot["heads"], dict) or len(snapshot["heads"]) > MAX_SERVICES:
            raise SystemError("Invalid snapshot heads")
        changed = []
        expected = {}
        for entry in items(snapshot["files"], MAX_SERVICES * MAX_ITEMS * 3 + MAX_ITEMS):
            fields(entry, ("service", "path", "sha256"))
            sid, path, sha = entry["service"], relative(entry["path"]), entry["sha256"]
            if not isinstance(sha, str) or not DIGEST.fullmatch(sha) or (sid, path) in expected:
                raise SystemError("Invalid or duplicate snapshot source")
            expected[(sid, path)] = sha
            if sid == "__system__":
                root = self.root
            elif sid in self.services and self.services[sid]["access"] == "available":
                root = self.services[sid]["root"]
            else:
                changed.append({"service": sid, "path": path, "reason": "unavailable"})
                continue
            try:
                if digest(read_file(root, path)) != sha:
                    changed.append({"service": sid, "path": path, "reason": "content_changed"})
            except (OSError, SystemError):
                changed.append({"service": sid, "path": path, "reason": "unavailable"})
        # Configuration and every readable manifest influenced scope selection.
        for key, sha in self.snapshot.items():
            if expected.get(key) != sha and not any((c["service"], c.get("path")) == key for c in changed):
                changed.append({"service": key[0], "path": key[1], "reason": "catalog_changed"})
        for entry in items(snapshot["missing"], MAX_SERVICES * MAX_ITEMS * 3 + MAX_ITEMS):
            fields(entry, ("service", "path"))
            sid, path = entry["service"], relative(entry["path"])
            if sid == "__system__":
                root = self.root
            elif sid in self.services and self.services[sid]["access"] == "available":
                root = self.services[sid]["root"]
            else:
                continue
            try:
                read_file(root, path)
            except (OSError, SystemError):
                continue
            changed.append({"service": sid, "path": path, "reason": "source_appeared"})
        for sid, head in snapshot["heads"].items():
            if sid not in self.services or (head is not None and
                    (not isinstance(head, str) or not re.fullmatch(r"[0-9a-f]{40,64}", head))):
                raise SystemError("Invalid snapshot service or commit")
            if self.services[sid]["head"] != head:
                changed.append({"service": sid, "reason": "head_changed"})
        context = plan.get("context")
        fields(context, ("task", "services", "sources"))
        for entry in items(context["sources"], MAX_SERVICES * MAX_ITEMS):
            if not isinstance(entry, dict):
                raise SystemError("Invalid saved context source")
            path, sid = relative(entry.get("path")), entry.get("service")
            if (sid, path) not in expected:
                raise SystemError("Context source has no snapshot fingerprint")
            if entry.get("kind") == "memory" or is_memory_path(path):
                if sid == "__system__":
                    root = self.root
                elif sid in self.services and self.services[sid]["access"] == "available":
                    root = self.services[sid]["root"]
                else:
                    continue
                if self.source_content(sid, root, {"path": path, "kind": "memory"}) is None:
                    changed.append({"service": sid, "path": path, "reason": "memory_ineligible"})
        return {"fresh": not changed, "changed": changed,
                "warnings": self.warnings,
                "meaning": "Source currency only; this does not approve a plan or verify its claims"}
