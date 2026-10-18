#!/usr/bin/env python3
"""Validate the canonical repository memory bank without external packages."""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import re
import subprocess
import sys
import threading
from datetime import date
from pathlib import Path
from typing import Optional



def _workspace_roots():
    """The attached-layout helper beside this file, or None.

    The validator also runs copied on its own - a staged bank's check - and
    then knows only the installed layout, which is all such a bank has.
    """
    try:
        import workspace_roots
    except ImportError:
        directory = str(Path(__file__).resolve().parent)
        sys.path.insert(0, directory)
        try:
            import workspace_roots
        except ImportError:
            return None
        finally:
            sys.path.remove(directory)
    return workspace_roots


workspace_roots = _workspace_roots()


# Chunk identifiers come in two accepted formats:
# - MEM-YYYYMMDD-xxxxxxxx (current): the allocation date plus eight hex
#   characters of the source Brain record's UUID. Needing no shared counter,
#   concurrent promotions on different machines or branches cannot collide.
# - MEM-0001 (legacy): sequential numbers once allocated from
#   `.memory-counter`. Existing chunks keep these IDs forever; nothing
#   renames them.
# The date-based alternative comes first so a filename match captures the
# whole date-based ID instead of stopping at its digit prefix.
# `.memory-counter` files may still exist on disk, but they are legacy, are
# no longer an ID source, and the validator deliberately ignores them.
ID_PATTERN = re.compile(r"^MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,})$")
FILENAME_PATTERN = re.compile(
    r"^(MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,}))-[a-z0-9]+(?:-[a-z0-9]+)*\.md$"
)
ALLOWED_TYPES = {
    "architecture",
    "constraint",
    "convention",
    "decision",
    "domain",
    "integration",
    "operations",
}
ALLOWED_STATUSES = {"active", "needs-review", "superseded", "archived"}
REQUIRED_KEYS = {
    "id",
    "title",
    "type",
    "status",
    "scope",
    "tags",
    "created",
    "last_verified",
    "review_after",
    "sources",
    "supersedes",
    "superseded_by",
}
# Temporal validity. Optional, because requiring it would invalidate every
# chunk written before it existed. `superseded_by` says what replaced a chunk;
# `valid_to` says when it stopped being true. Automatically written memory
# needs the second: a wrong fact earns a boundary instead of being erased.
OPTIONAL_KEYS = {
    "valid_from",
    "valid_to",
    # Digests of the files this chunk cites, so a chunk can notice that the
    # thing it summarizes has moved on. Optional, because requiring it would
    # invalidate every chunk written before it existed - the same reason
    # `valid_from`/`valid_to` are optional.
    "source_digests",
}
# What follows a credential key when the value is not a literal credential: a
# variable or expression, an env/config lookup, a template, a placeholder, a
# validation rule, a mask, or a bare number. Without it the pattern refused
# ordinary PHP and Symfony text (`$password = $request->validated(...)`,
# `secret: '%env(APP_SECRET)%'`) and dropped whole skills from the index over
# a documented `--secret=...` example.
def _automatic_query():
    """The secret and personal-data patterns, from the module beside this file.

    Loaded the way workspace_roots is, so a validator loaded by its path rather
    than from its own directory still finds them. Without them nothing could
    be checked, so a runtime missing the module says so instead of passing.
    """
    try:
        import automatic_query
    except ImportError:
        directory = str(Path(__file__).resolve().parent)
        sys.path.insert(0, directory)
        try:
            import automatic_query
        except ImportError as error:
            raise ImportError(
                "memory-bank/scripts/automatic_query.py is missing beside validate.py; "
                "reinstall or sync the memory runtime"
            ) from error
        finally:
            sys.path.remove(directory)
    return automatic_query


_automatic = _automatic_query()
PRIVATE_PATTERNS = _automatic.PRIVATE_PATTERNS
SECRET_PATTERNS = _automatic.SECRET_PATTERNS
QUOTED_CREDENTIAL = _automatic.QUOTED_CREDENTIAL
PRIVATE_KEY_BLOCK = _automatic.PRIVATE_KEY_BLOCK
AutomaticQueryError = _automatic.AutomaticQueryError
_sanitize_automatic_query = _automatic.sanitize_automatic_query

def sensitive_label(text: str) -> Optional[str]:
    """What sensitive data `text` appears to carry, or None.

    Names the kind only - never the matched value - so the message can be
    shown, logged and put into a capsule without repeating what it refuses.
    """
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(text):
            return f"a possible {label}"
    for label, pattern in PRIVATE_PATTERNS.items():
        if pattern.search(text):
            return f"personal data ({label})"
    return None


class ValidationError(Exception):
    """A memory-bank contract violation."""


def sanitize_automatic_query(text: str) -> str:
    try:
        return _sanitize_automatic_query(text)
    except AutomaticQueryError as error:
        raise ValidationError(str(error)) from error


def display_path(path: Path, bank_root: Path) -> str:
    """A bank path as a message reader should see it: from the bank's parent.

    Validation messages travel further than the terminal that ran them - a
    refused promotion's reason lands in the next Task Capsule - and an
    absolute path carries the machine's home directory into a prompt while
    saying nothing `memory-bank/chunks/...` does not.
    """
    try:
        return path.relative_to(bank_root.parent).as_posix()
    except ValueError:
        return str(path)


def parse_frontmatter(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        raise ValidationError("missing opening frontmatter delimiter")
    try:
        raw_metadata, _ = text[4:].split("\n---\n", 1)
    except ValueError as error:
        raise ValidationError("missing closing frontmatter delimiter") from error
    try:
        metadata = json.loads(raw_metadata)
    except json.JSONDecodeError as error:
        raise ValidationError(f"frontmatter is not valid JSON-compatible YAML: {error}") from error
    if not isinstance(metadata, dict):
        raise ValidationError("frontmatter must be an object")
    return metadata


def require_string_list(metadata: dict, key: str, *, allow_empty: bool = False) -> None:
    value = metadata.get(key)
    if not isinstance(value, list) or any(not isinstance(item, str) or not item for item in value):
        raise ValidationError(f"{key} must be a list of non-empty strings")
    if not allow_empty and not value:
        raise ValidationError(f"{key} must not be empty")


def require_date(metadata: dict, key: str) -> date:
    value = metadata.get(key)
    if not isinstance(value, str):
        raise ValidationError(f"{key} must be an ISO date string")
    try:
        return date.fromisoformat(value)
    except ValueError as error:
        raise ValidationError(f"{key} must use YYYY-MM-DD") from error


def memory_temporal_reason(metadata: dict) -> Optional[str]:
    """Current expiry of declared bounds, using one calendar observation.

    This is an admission recheck, not a replacement for full chunk validation.
    Older raw SQL rows without declared bounds retain their existing contract;
    every chunk admitted by normal discovery has a required review boundary.
    """
    today = date.today()
    bounds = {}
    for key in ("review_after", "valid_to"):
        if key not in metadata or (key == "valid_to" and metadata[key] is None):
            continue
        try:
            bounds[key] = require_date(metadata, key)
        except ValidationError:
            return "invalid"
    if "valid_to" in bounds and today > bounds["valid_to"]:
        return "retired"
    if "review_after" in bounds and today > bounds["review_after"]:
        return "overdue-review"
    return None


def terminal_chunk(metadata: dict, valid_to: object) -> bool:
    """Whether a chunk has reached the end of its life, by status or by date.

    A terminal chunk no longer answers questions, so the sources it cites are
    history rather than obligations. Deleting a cited file is routine work -
    dropping a controller, regenerating a schema - and it must not make an
    unrelated task fail validation for a chunk that was correctly archived.
    """
    if metadata.get("status") in {"superseded", "archived"}:
        return True
    return valid_to is not None and valid_to < date.today()


_SOURCE_FACTS = threading.local()


@contextlib.contextmanager
def source_fact_memo():
    """Within one read-only pass over the Memory Bank, look at each cited
    source once: thousands of chunks citing the same files resolved and
    checked them again chunk by chunk, on every prompt. The answers live only
    inside the `with`, where no file is created, moved or removed."""
    previous = getattr(_SOURCE_FACTS, "memo", None)
    _SOURCE_FACTS.memo = {}
    try:
        yield
    finally:
        _SOURCE_FACTS.memo = previous


def _source_facts(repository_root: Path, source_path: str) -> tuple[bool, bool]:
    """Whether a cited source lies inside the repository, and exists."""
    memo = getattr(_SOURCE_FACTS, "memo", None)
    key = (os.fspath(repository_root), source_path)
    if memo is not None and key in memo:
        return memo[key]
    if workspace_roots is not None:
        resolved_source = workspace_roots.resolve(repository_root, source_path).resolve()
        inside = workspace_roots.contains(repository_root, resolved_source)
    else:
        resolved_source = (repository_root / source_path).resolve()
        inside = resolved_source.is_relative_to(repository_root.resolve())
    facts = (inside, inside and resolved_source.exists())
    if memo is not None:
        memo[key] = facts
    return facts


def validate_metadata(
    path: Path,
    metadata: dict,
    repository_root: Path,
    warnings: Optional[list[str]] = None,
    label: Optional[str] = None,
) -> None:
    """Raise on anything that makes a chunk unusable.

    ``warnings`` opts a caller into the non-fatal class: a terminal chunk
    whose cited source has since been deleted is reported there instead of
    raised. Without a sink the behaviour is unchanged, which is what keeps
    the retrieval gate in `active_memory` strict. ``label`` names the chunk
    in such a warning; it defaults to ``path``.
    """
    missing = REQUIRED_KEYS - metadata.keys()
    extra = metadata.keys() - REQUIRED_KEYS - OPTIONAL_KEYS
    if missing:
        raise ValidationError(f"missing metadata keys: {', '.join(sorted(missing))}")
    if extra:
        raise ValidationError(f"unexpected metadata keys: {', '.join(sorted(extra))}")

    filename_match = FILENAME_PATTERN.fullmatch(path.name)
    if filename_match is None:
        raise ValidationError(
            "filename must be MEM-YYYYMMDD-xxxxxxxx-short-slug.md "
            "(or legacy MEM-0001-short-slug.md)"
        )
    memory_id = metadata["id"]
    if not isinstance(memory_id, str) or ID_PATTERN.fullmatch(memory_id) is None:
        raise ValidationError(
            "id must use MEM-YYYYMMDD-xxxxxxxx (or legacy MEM-0001) format"
        )
    if filename_match.group(1) != memory_id:
        raise ValidationError("frontmatter id does not match filename id")
    if not isinstance(metadata["title"], str) or not metadata["title"].strip():
        raise ValidationError("title must be a non-empty string")
    if not isinstance(metadata["type"], str) or metadata["type"] not in ALLOWED_TYPES:
        raise ValidationError(f"type must be one of: {', '.join(sorted(ALLOWED_TYPES))}")
    if not isinstance(metadata["status"], str) or metadata["status"] not in ALLOWED_STATUSES:
        raise ValidationError(f"status must be one of: {', '.join(sorted(ALLOWED_STATUSES))}")

    require_string_list(metadata, "scope")
    require_string_list(metadata, "tags")
    require_string_list(metadata, "sources")
    require_string_list(metadata, "supersedes", allow_empty=True)
    created = require_date(metadata, "created")
    last_verified = require_date(metadata, "last_verified")
    review_after = require_date(metadata, "review_after")
    if created > last_verified:
        raise ValidationError("created must not be later than last_verified")
    if last_verified > date.today():
        raise ValidationError("last_verified must not be in the future")
    if review_after < last_verified:
        raise ValidationError("review_after must not be earlier than last_verified")
    if metadata["status"] == "active" and review_after < date.today():
        raise ValidationError("active chunk is overdue for review")

    valid_from = require_date(metadata, "valid_from") if "valid_from" in metadata else None
    valid_to = None
    if metadata.get("valid_to") is not None:
        valid_to = require_date(metadata, "valid_to")
    # valid_from may precede `created`: knowledge is often true well before
    # anyone writes it down.
    if valid_to is not None:
        if valid_from is not None and valid_to < valid_from:
            raise ValidationError("valid_to must not be earlier than valid_from")
        # Matches the review_after rule above: knowledge that has stopped being
        # true may not keep calling itself active.
        if metadata["status"] == "active" and valid_to < date.today():
            raise ValidationError("active chunk is past its valid_to date")

    digests = metadata.get("source_digests")
    if digests is not None:
        if not isinstance(digests, list):
            raise ValidationError("source_digests must be a list")
        for entry in digests:
            if not isinstance(entry, dict) or set(entry) != {"path", "sha256"}:
                raise ValidationError(
                    "each source_digests entry must be {path, sha256}"
                )
            if not isinstance(entry["path"], str) or not entry["path"]:
                raise ValidationError("source_digests path must be a non-empty string")
            if not isinstance(entry["sha256"], str) or not re.fullmatch(
                r"[0-9a-f]{64}", entry["sha256"]
            ):
                raise ValidationError("source_digests sha256 must be a hex digest")
        cited = {
            source.split("#", 1)[0]
            for source in metadata["sources"]
            if not source.startswith(("https://", "http://"))
        }
        if {entry["path"] for entry in digests} - cited:
            raise ValidationError(
                "source_digests may only cover paths listed in sources"
            )

    replacement = metadata["superseded_by"]
    if replacement is not None and (
        not isinstance(replacement, str) or ID_PATTERN.fullmatch(replacement) is None
    ):
        raise ValidationError("superseded_by must be null or a memory ID")
    if metadata["status"] == "superseded" and replacement is None:
        raise ValidationError("superseded chunks require superseded_by")
    if metadata["status"] != "superseded" and replacement is not None:
        raise ValidationError("only superseded chunks may set superseded_by")

    for source in metadata["sources"]:
        if source.startswith(("https://", "http://")):
            continue
        source_path = source.split("#", 1)[0]
        inside, exists = _source_facts(repository_root, source_path)
        if not inside:
            raise ValidationError(f"source path escapes the repository: {source_path}")
        if not exists:
            # The containment check above stays fatal in every status: it is a
            # boundary guarantee, not a freshness one.
            if warnings is not None and terminal_chunk(metadata, valid_to):
                warnings.append(
                    f"{label or path}: cited source no longer exists: {source_path} "
                    f"(chunk is {metadata['status']} and no longer retrievable)"
                )
                continue
            raise ValidationError(f"source path does not exist: {source_path}")


def parse_index(path: Path) -> dict[str, dict[str, str]]:
    rows: dict[str, dict[str, str]] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith("| MEM-"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) != 8:
            raise ValidationError(f"invalid index row: {line}")
        memory_id, title, memory_type, scope, tags, status, verified, file_path = cells
        if memory_id in rows:
            raise ValidationError(f"duplicate index ID: {memory_id}")
        rows[memory_id] = {
            "title": title,
            "type": memory_type,
            "scope": scope,
            "tags": tags,
            "status": status,
            "last_verified": verified,
            "file": file_path.strip("[]").split("](", 1)[-1].rstrip(")"),
        }
    return rows


# Files this process found free of every secret pattern, by path and file
# identity (device, inode, size, modification and change times). Promotion
# validates the whole bank before and after each chunk it writes, and every
# pass searched every chunk again.
_SECRET_FREE: dict[str, tuple[int, ...]] = {}


def validate_secret_patterns(path: Path) -> None:
    try:
        image = path.stat()
        key: Optional[tuple[int, ...]] = (image.st_dev, image.st_ino, image.st_size,
                                          image.st_mtime_ns, image.st_ctime_ns)
    except OSError:
        key = None
    if key is not None and _SECRET_FREE.get(str(path)) == key:
        return
    text = path.read_text(encoding="utf-8")
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(text):
            raise ValidationError(f"possible {label} detected; value intentionally not printed")
    if key is not None:
        if len(_SECRET_FREE) >= 50000:
            _SECRET_FREE.clear()
        _SECRET_FREE[str(path)] = key


# A match that opens with a key and a separator keeps the key - it is the
# project's vocabulary (MAILER_PASSWORD, APP_KEY) - and loses only its value.
_KEYED_SECRETS = frozenset(
    {"assigned credential", "Laravel application key", "authorization header", "basic credentials"}
)
_KEY_SEPARATOR = re.compile(r"[:=][ \t]*")
_VALUE_DELIMITERS = "'\"`"
MASK_PASSES = 3
# Bump when the masking rule itself changes; the patterns are fingerprinted
# from their own source. Either change makes the index re-read every document.
MASK_POLICY_VERSION = "1"


def secret_policy_fingerprint() -> str:
    """A digest of what decides an indexed document's masked text.

    The stat cache keeps a document unread while its file is unchanged, so
    without this a pattern added later would never reach a document indexed
    before it - and after masking, the index is where secrets must not be.
    """
    parts = [MASK_POLICY_VERSION, PRIVATE_KEY_BLOCK.pattern, QUOTED_CREDENTIAL.pattern,
             *sorted(_KEYED_SECRETS)]
    for label, pattern in SECRET_PATTERNS.items():
        parts += [label, pattern.pattern, str(pattern.flags)]
    return hashlib.sha256("\0".join(parts).encode("utf-8")).hexdigest()


def _masked_span(text: str, label: str, match: "re.Match[str]") -> tuple[int, int]:
    """The stretch of `text` that carries the secret `match` found."""
    if label == "private key":
        # The header alone is detected; the body under it is the secret.
        block = PRIVATE_KEY_BLOCK.match(text, match.start())
        return match.start(), (block.end() if block else match.end())
    separator = (
        _KEY_SEPARATOR.search(text, match.start(), match.end())
        if label in _KEYED_SECRETS else None
    )
    if separator is None:
        # Self-identifying tokens, URL credentials, JWTs, bearer tokens.
        return match.span()
    if label == "assigned credential":
        quoted = QUOTED_CREDENTIAL.match(text, match.start())
        if quoted is not None:
            # The whole quoted literal, to its closing quote or the line's end.
            return quoted.start(1), quoted.end()
    if match.end() < len(text) and text[match.end()] in _VALUE_DELIMITERS:
        # A value closed by inline code or a quote: `MAIL_PASSWORD=x` in prose.
        return separator.end(), match.end()
    # Unquoted: the rest of the line, so no word of a multi-word value stays.
    line_end = text.find("\n", match.end())
    return separator.end(), (len(text) if line_end < 0 else line_end)


# Necessary ASCII literals for these exact source/flags profiles. A changed
# or unknown pattern keeps the full scan; this never decides what is secret.
_MASK_PREFILTERS = {
    'private key': ('42d5f36d732a5e75d1a8e481fda6c7a226948c56a005eb54e2ca65c3fd34156d', 32, ('-----begin ',)),
    'GitHub token': ('68761294102b638abad4a26260391647743fa10bb874d45c6159f572cd96de7a', 32, ('ghp_', 'gho_', 'ghu_', 'ghs_', 'ghr_', 'github_pat_')),
    'GitLab token': ('6cf7ed0fce6679d0e13564578f16dfece2fe3fc29729d876ceef111abf9f43eb', 32, ('gl',)),
    'Slack token': ('5ad38f6c78f86ce64f20a423704ec912647b1ec9efc40c7cf1599ba17ee007e1', 32, ('xox', 'xapp-')),
    'Slack webhook': ('f40b6113132ea8a516c11e934688b099d79e909cedcfa1184eb923bd6e7aceec', 34, ('hooks.slack.com/',)),
    'Discord webhook': ('b0d1d794dbbbf3d1a3d0dbee5e39bf5588ffac8c7da4f405ec37a5d27bfdfb12', 34, ('.com/api/webhooks/',)),
    'Telegram bot token': ('0d73b9142c78a8f6d9bd69c95f15a64a9e272ca7c6dcadb8ac5b3c425ce16959', 32, (':aa',)),
    'AWS access key': ('7f8a678753a46ba3f6704958b8320cba8e038fd7fb8d8cb8e0c173db25ea24f9', 32, ('akia', 'asia')),
    'Google API key': ('34494ea7dde5044d930ce9fd926f1c23e7b36e5197c42007728ba26a1ae4dcd9', 32, ('aiza',)),
    'Google OAuth token': ('429afb0689fd198c3bf36fc2a1210aa9288b9010a916efa78f55cb4f39224c65', 32, ('ya29.',)),
    'OpenAI-style token': ('1fcad04827f5b98cc5d41309369d1802d3639e23f4d35eddac1a93b52c009809', 32, ('sk-',)),
    'Stripe secret key': ('a6b6ed7945eefeb3695a7477b82f30c6c0b0abff91a14dd7a9f52e6cbb4c86ff', 32, ('rk_', 'sk_')),
    'npm token': ('3d97ff554ee799bff07ff452c2a5327654ec2fb76d7d92c1c625ab5318063983', 32, ('npm_',)),
    'PyPI token': ('f2edca40d579d9ccb9b07ae016b754d34086235f9903e03eff15fa2d586e964b', 32, ('pypi-age',)),
    'Hugging Face token': ('a0b78eeb650697addc2777dbf7554a95ebe8098be27b4e8622c467b9e22d8091', 32, ('hf_',)),
    'SendGrid key': ('3de13be719f8c8a1db81a233d44c9d75b3e9781564bc49fbff3d775b4b83f994', 32, ('sg.',)),
    'Shopify token': ('aaea7614b9095bdafc9d0305256b1f029f42bc19cb97cc25b927f0bec50d78f9', 32, ('shp',)),
    'DigitalOcean token': ('5ef55bfd23ba33d17e5788cdc297e1744f2d4894b87d23053d28c17588ce0b6c', 32, ('_v1_',)),
    'JWT': ('d190c7a28bb6084cfc4bfafd2d67ff1a8781b83e38ce367267a5f4ecd2c5c875', 32, ('eyj',)),
    'Laravel application key': ('c291ae904226de167e2c62f924a58e32c809f577e0e7750021a68498773e3b9c', 32, ('app_key',)),
    'credential in URL': ('633cc88e42dc82836c82d03949215d6d65fbaff1d1cea0b8249131ddb793acc5', 34, ('://',)),
    'authorization header': ('a30545248891aef90bec6fbd535e17f6b391bc5fe49cb8e76cf5492e8ea48fb1', 34, ('authorization',)),
    'basic credentials': ('caf3189272465291e6592220028ef036076b261109ac5ee486603e0ca67a1cd8', 34, ('authorization',)),
    'bearer token': ('ee8f758f0d60a50dd2c85bb10a910c7b2fd8200fab3ec9b448b807d34afe3030', 34, ('bearer',)),
    'assigned credential': ('0f981eb49ea9ecedab5fa894792b7ddd1d5adb1821ab23d6b444ac27fa6922dd', 34, ('password', 'passwd', 'secret', 'apikey', 'api_key', 'api-key', 'accesstoken', 'accesskey', 'access_token', 'access_key', 'access-token', 'access-key', 'authtoken', 'auth_token', 'auth-token', 'refreshtoken', 'refresh_token', 'refresh-token', 'privatekey', 'private_key', 'private-key', 'accountkey', 'account_key', 'account-key', 'sharedaccesskey', 'sharedaccess_key', 'sharedaccess-key', 'shared_accesskey', 'shared_access_key', 'shared_access-key', 'shared-accesskey', 'shared-access_key', 'shared-access-key')),
}
_MASK_ORIGINAL_PATTERNS = {
    label: (pattern, _MASK_PREFILTERS[label][2])
    for label, pattern in SECRET_PATTERNS.items()
    if label in _MASK_PREFILTERS
    and type(pattern) is re.Pattern
    and type(pattern.pattern) is str
    and pattern.flags == _MASK_PREFILTERS[label][1]
    and hashlib.sha256(pattern.pattern.encode("utf-8")).hexdigest()
        == _MASK_PREFILTERS[label][0]
}


def _mask_pattern_candidates(text: str):
    """Skip only impossible exact-pattern matches, retaining original order.

    Unicode IGNORECASE also admits dotless/dotted i, long s and Kelvin.
    This view is deliberately overinclusive; every actual regex still sees
    the original text and computes its spans there. Recompute after each
    masking pass because a replacement may introduce a subsequent cue.
    """
    folded = text.casefold().replace("\u0131", "i").replace("\u0307", "")
    for label, pattern in SECRET_PATTERNS.items():
        original = _MASK_ORIGINAL_PATTERNS.get(label)
        if (original is None or pattern is not original[0]
                or any(cue in folded for cue in original[1])):
            yield label, pattern


# The full key grammar of the pinned assigned-credential policy. Do not use
# the looser cue list here: these positions are only possible regex anchors.
_ASSIGNED_KEY_START = re.compile(
    r"(?<![A-Za-z0-9])" + '(?:password|passwd|secret(?:_access)?(?:_key|_token)?|api[_-]?key|access[_-]?(?:token|key)|auth[_-]?token|refresh[_-]?token|private[_-]?key|account[_-]?key|shared[_-]?access[_-]?key)' + r"[ \t]*[:=]",
    re.IGNORECASE,
)


def _assigned_prefix_character(character: str) -> bool:
    return (
        "a" <= character <= "z" or "A" <= character <= "Z"
        or "0" <= character <= "9" or character in "\u0130\u0131\u017f\u212a"
    )


def _assigned_mask_profile(label, pattern, text) -> bool:
    original = _MASK_ORIGINAL_PATTERNS.get("assigned credential")
    return (label == "assigned credential" and original is not None
            and pattern is original[0] and type(text) is str)


def _assigned_mask_matches(pattern, text):
    """Original consumed spans without retrying every valid prefix suffix.

    Keep this private: anchored Match.pos differs from finditer's, whereas
    the masking caller consumes only the original start/end/span methods.
    Value validation is still the original regex and can have its own cost.
    """
    resume = 0
    for candidate in _ASSIGNED_KEY_START.finditer(text):
        start = candidate.start()
        if start < resume:
            continue
        while start > resume and text[start - 1] == "_":
            segment_end = start - 1
            segment_start = segment_end
            while (segment_start > resume
                   and _assigned_prefix_character(text[segment_start - 1])):
                segment_start -= 1
            if segment_start == segment_end:
                # A double underscore is not an ALNUM+_ prefix segment.
                break
            if (segment_start == resume and resume > 0
                    and _assigned_prefix_character(text[resume - 1])):
                # The search resumed inside a word; its clipped beginning
                # cannot satisfy the original negative lookbehind.
                break
            start = segment_start
        found = pattern.match(text, start)
        if found is not None:
            yield found
            # A quoted value may contain more original matches even when the
            # mask later expands to its closing quote. Resume at the regex end.
            resume = found.end()


def _mask_pattern_matches(label, pattern, text):
    if _assigned_mask_profile(label, pattern, text):
        yield from _assigned_mask_matches(pattern, text)
    else:
        yield from pattern.finditer(text)


def _mask_pattern_search(label, pattern, text):
    if _assigned_mask_profile(label, pattern, text):
        return next(_assigned_mask_matches(pattern, text), None)
    return pattern.search(text)


def indexed_secret_policy_reason(*texts: str) -> Optional[str]:
    """Whether a held indexed projection needs a new privacy pass.

    Admission must not remask an old capsule into apparent validity. Ordinary
    sources can intentionally contain values already masked in their indexed
    image; check that image, using the existing expression/template semantics.
    Strict structured authority callers also pass their held original source.
    """
    for text in texts:
        if not isinstance(text, str):
            return "secret-policy"
        try:
            masked, _ = mask_secrets(text)
        except ValidationError:
            return "secret-policy"
        if masked != text:
            return "secret-policy"
    return None


def mask_secrets(text: str) -> tuple[str, int]:
    """`text` with each value SECRET_PATTERNS recognises replaced by
    `[redacted: <label>]`, and how many values were replaced.

    Text with no detection comes back unchanged with 0, so a document indexed
    before masking existed is indexed byte for byte as it was. Personal data
    (PRIVATE_PATTERNS) is not masked here: the render screen removes it from
    capsule text. Raises ValidationError, never naming the value, when a
    pattern still matches after MASK_PASSES passes: the caller then leaves
    the document out, as it used to leave out every document with a match.
    """
    masked = 0
    for _ in range(MASK_PASSES):
        spans = sorted(
            (*_masked_span(text, label, found), label)
            for label, pattern in _mask_pattern_candidates(text)
            for found in _mask_pattern_matches(label, pattern, text)
        )
        if not spans:
            return text, masked
        pieces: list[str] = []
        end = 0
        for start, stop, label in spans:
            if start < end:
                # Overlaps a value already masked: widen it, no second marker.
                end = max(end, stop)
                continue
            pieces += [text[end:start], f"[redacted: {label}]"]
            end = stop
            masked += 1
        pieces.append(text[end:])
        text = "".join(pieces)
    if any(_mask_pattern_search(label, pattern, text)
           for label, pattern in _mask_pattern_candidates(text)):
        raise ValidationError("possible secret survives masking; value intentionally not printed")
    return text, masked


def moved_to(project_root: Path, source: str) -> Optional[str]:
    """Where Git moved a cited file that is gone - the end of its rename
    chain, when that file exists - or None. For the person repointing a
    chunk; the per-prompt checks never start Git for this."""
    environment = {key: value for key, value in os.environ.items()
                   if key not in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR")}
    current = source
    for _ in range(5):
        try:
            result = subprocess.run(
                ["git", "-C", str(project_root), "log", "-M", "--diff-filter=R", "--name-status",
                 "--format=", "-n", "1", "--full-diff", "--", current],
                capture_output=True, text=True, timeout=5, env=environment, check=False)
        except (OSError, subprocess.SubprocessError, ValueError):
            return None
        fields = [line.split("\t") for line in result.stdout.splitlines() if line.startswith("R")]
        target = next((parts[2] for parts in fields if len(parts) == 3 and parts[1] == current), None)
        if result.returncode != 0 or target is None:
            break
        current = target
        if (project_root / current).is_file():
            return current
    return None


_MISSING_SOURCE = re.compile(r"source path does not exist: (.+)$")


def summarize_bank(bank_root: Path) -> str:
    chunks_dir = bank_root / "chunks"
    total = 0
    active = 0
    needs_review = 0
    expired = 0
    # An active chunk whose cited file was moved or deleted is no longer
    # retrieved, and the summary went on counting it as active: renaming a
    # file with `git mv` took its knowledge out of every capsule unseen.
    gone = 0
    if chunks_dir.is_dir():
        with source_fact_memo():
            for path in sorted(chunks_dir.iterdir()):
                if path.is_symlink() or not path.is_file() or FILENAME_PATTERN.fullmatch(path.name) is None:
                    continue
                total += 1
                try:
                    metadata = parse_frontmatter(path)
                    status = metadata.get("status")
                    if isinstance(metadata.get("valid_to"), str):
                        try:
                            if date.fromisoformat(metadata["valid_to"]) < date.today():
                                expired += 1
                        except ValueError:
                            pass
                except (OSError, UnicodeDecodeError, ValidationError):
                    continue
                active += status == "active"
                needs_review += status == "needs-review"
                if status == "active":
                    sources = metadata.get("sources")
                    try:
                        gone += any(isinstance(source, str) and _source_facts(bank_root.parent, source) == (True, False)
                                    for source in (sources if isinstance(sources, list) else []))
                    except (OSError, ValueError):
                        pass
    summary = f"Memory bank: {total} chunks ({active} active, {needs_review} needs review"
    summary += f", {expired} past valid_to)." if expired else ")."
    if gone:
        summary += (f" {gone} active chunk(s) cite a file that is gone and are not retrieved;"
                    " memory-bank/scripts/validate.py names them.")
    return summary


def validate_bank(bank_root: Path, source_root: Path | None = None) -> list[str]:
    """Every problem that makes the bank invalid. Kept for existing callers."""
    return validate_bank_report(bank_root, source_root)[0]


def validate_bank_report(
    bank_root: Path, source_root: Path | None = None
) -> tuple[list[str], list[str]]:
    """Errors and warnings, separately.

    The split exists because two different situations produced the same red
    output: a chunk that is wrong, and a chunk that correctly reached the end
    of its life while a file it cites was deleted. Only the first is something
    the engineer must fix, and only the first should fail a task that has
    nothing to do with the bank.
    """
    errors: list[str] = []
    warnings: list[str] = []
    # Chunks cite the project's own files, which sit beside the bank once it is
    # published. During generation the bank is staged apart from the project it
    # describes, so the caller passes the tree those citations resolve against;
    # without it a correctly seeded bank fails purely for being staged.
    repository_root = (source_root or bank_root.parent).resolve()
    index_path = bank_root / "INDEX.md"
    chunks_dir = bank_root / "chunks"

    def shown(path: Path) -> str:
        return display_path(path, bank_root)

    index_label = shown(index_path)
    # `.memory-counter` is intentionally absent from the required files and
    # from every check below: identifiers are date+UUID based now, so the
    # counter is a retired legacy artifact that may or may not exist on disk.
    for required in (bank_root / "README.md", index_path):
        if not required.is_file():
            errors.append(f"{shown(required)}: required file is missing")
    if errors:
        return errors, warnings

    try:
        index = parse_index(index_path)
    except UnicodeDecodeError:
        errors.append(f'{index_label}: invalid UTF-8 index')
        index = {}
    except ValidationError as error:
        errors.append(f"{index_label}: {error}")
        index = {}

    chunk_paths: list[Path] = []
    if chunks_dir.is_dir():
        for entry in sorted(chunks_dir.iterdir()):
            if entry.is_symlink() or not entry.is_file() or FILENAME_PATTERN.fullmatch(entry.name) is None:
                errors.append(
                    f"{shown(entry)}: unexpected chunk entry; expected a direct MEM-0001-short-slug.md file"
                )
                continue
            chunk_paths.append(entry)

    chunks: dict[str, tuple[Path, dict]] = {}
    # Every id that exists on disk, including chunks that failed validation.
    # The index cross-check below distinguishes "the file is gone" from "the
    # file is here and broken", and a chunk can fail before its id is
    # readable, so the filename is the fallback.
    on_disk: set[str] = set()
    for path in chunk_paths:
        filename_match = FILENAME_PATTERN.fullmatch(path.name)
        if filename_match is not None:
            on_disk.add(filename_match.group(1))
        try:
            metadata = parse_frontmatter(path)
            validate_metadata(
                path, metadata, repository_root, warnings, label=shown(path)
            )
            validate_secret_patterns(path)
            memory_id = metadata["id"]
            on_disk.add(memory_id)
            if memory_id in chunks:
                raise ValidationError(f"duplicate chunk ID: {memory_id}")
            chunks[memory_id] = (path, metadata)
        except UnicodeDecodeError:
            errors.append(f'{shown(path)}: invalid UTF-8 chunk')
        except (OSError, ValidationError) as error:
            errors.append(f"{shown(path)}: {error}")

    for memory_id, (path, metadata) in chunks.items():
        row = index.get(memory_id)
        if row is None:
            errors.append(f"{shown(path)}: chunk is missing from INDEX.md")
            continue
        # INDEX.md names chunks with '/', as render_bank_index writes them.
        expected_file = path.relative_to(bank_root).as_posix()
        if row["file"] != expected_file:
            errors.append(f"{index_label}: {memory_id} file must be {expected_file}")
        for index_key, metadata_key in (
            ("title", "title"),
            ("type", "type"),
            ("status", "status"),
            ("last_verified", "last_verified"),
        ):
            if row[index_key] != str(metadata[metadata_key]):
                errors.append(f"{index_label}: {memory_id} {index_key} differs from chunk metadata")
        for index_key, metadata_key in (("scope", "scope"), ("tags", "tags")):
            expected = ", ".join(metadata[metadata_key])
            if row[index_key] != expected:
                errors.append(f"{index_label}: {memory_id} {index_key} differs from chunk metadata")

    for memory_id, row in index.items():
        if memory_id in chunks:
            continue
        if memory_id in on_disk:
            # The chunk is on disk and already reported its own error above.
            # Saying it is missing sends the reader looking for a deleted file
            # that is sitting right there.
            warnings.append(
                f"{index_label}: {memory_id} index row retained for invalid chunk "
                f"({row['file']})"
            )
        else:
            errors.append(
                f"{index_label}: {memory_id} points to a missing chunk ({row['file']})"
            )

    for memory_id, (_, metadata) in chunks.items():
        if memory_id in metadata["supersedes"] or metadata["superseded_by"] == memory_id:
            errors.append(f"{memory_id}: chunk cannot supersede itself")
        referenced = [*metadata["supersedes"]]
        if metadata["superseded_by"] is not None:
            referenced.append(metadata["superseded_by"])
        for referenced_id in referenced:
            if referenced_id not in chunks:
                errors.append(f"{memory_id}: replacement link points to missing {referenced_id}")

        for superseded_id in metadata["supersedes"]:
            if superseded_id not in chunks:
                continue
            superseded_metadata = chunks[superseded_id][1]
            if superseded_metadata["status"] != "superseded":
                errors.append(f"{memory_id}: {superseded_id} must have superseded status")
            if superseded_metadata["superseded_by"] != memory_id:
                errors.append(f"{memory_id}: {superseded_id} must point back with superseded_by")

        replacement_id = metadata["superseded_by"]
        if replacement_id in chunks and memory_id not in chunks[replacement_id][1]["supersedes"]:
            errors.append(f"{memory_id}: {replacement_id} must include this ID in supersedes")

    for start_id in chunks:
        visited: set[str] = set()
        current_id: str | None = start_id
        while current_id in chunks:
            if current_id in visited:
                errors.append(f"{start_id}: supersession cycle detected")
                break
            visited.add(current_id)
            current_id = chunks[current_id][1]["superseded_by"]

    return errors, warnings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "bank",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="Memory-bank directory (default: directory containing this script)",
    )
    parser.add_argument(
        "--summary",
        action="store_true",
        help="print status counts from chunk frontmatter without printing chunk contents",
    )
    parser.add_argument(
        "--source-root",
        type=Path,
        default=None,
        help=(
            "tree the chunk sources resolve against (default: the bank's parent). "
            "Pass the real project root when validating a staged bank."
        ),
    )
    args = parser.parse_args()
    bank_root = args.bank.resolve()
    if args.summary:
        print(summarize_bank(bank_root))
        return 0

    errors, warnings = validate_bank_report(
        bank_root, args.source_root.resolve() if args.source_root else None
    )
    # A chunk citing a file renamed with `git mv` is left out of every
    # capsule; say where the file went, so the chunk can be repointed.
    source_root = args.source_root.resolve() if args.source_root else bank_root.parent
    project_root = workspace_roots.project_root(source_root) if workspace_roots is not None else source_root

    def located(line: str) -> str:
        missing = _MISSING_SOURCE.search(line)
        target = moved_to(project_root, missing.group(1).strip()) if missing else None
        return line + (f" - Git moved it to {target}" if target else "")

    errors = [located(error) for error in errors]
    warnings = [located(warning) for warning in warnings]
    if warnings:
        # Printed on both paths and never fatal: a warning describes a chunk
        # that is doing what it promised, so it must not fail a task that has
        # nothing to do with the bank.
        print(f"Memory bank validation warnings ({len(warnings)}):", file=sys.stderr)
        for warning in warnings:
            print(f"- {warning}", file=sys.stderr)
    if errors:
        print(f"Memory bank validation failed ({len(errors)} error(s)):", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("Memory bank validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
