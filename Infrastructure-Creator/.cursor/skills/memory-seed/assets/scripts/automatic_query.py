"""Pure automatic-query adapter shared by memory hooks and Harness (stdlib only)."""
from __future__ import annotations

import re

_NOT_A_LITERAL = (
    r"(?!"
    r"[$%{<\[(=]"
    r"|(?:get)?env\(|config\(|secret\(|process\.env|os\.environ|vault:"
    r"|\*{2,}|x{3,}|\.{2,}|\u2026"
    r"|\d+(?![^\s'\"`,;])"
    r"|[A-Za-z_\\][\w\\.]*(?:::|->|\()"
    r"|[a-z_]\w*\.[a-z_][\w.]*(?![^\s'\"`,;])"
    r"|(?:required|nullable|sometimes|confirmed|hashed|string|null|none|true|false"
    r"|secret|password|passw(?:or)?d|pass|root|test|example|placeholder|redacted"
    r"|!?change[-_]?me!?|your[-_ ][^\s]*)(?![A-Za-z0-9])"
    r")"
)
SECRET_PATTERNS = {
    "private key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "GitHub token": re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b"),
    "AWS access key": re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
    "OpenAI-style token": re.compile(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b"),
    "Stripe secret key": re.compile(r"\bsk_(?:live|test)_[A-Za-z0-9]{16,}\b"),
    "JWT": re.compile(r"\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b"),
    "Laravel application key": re.compile(
        r"\bAPP_KEY[ \t]*=[ \t]*['\"]?base64:[A-Za-z0-9+/]{20,}={0,2}"
    ),
    # user:password@ in a URL or DSN (mysql://, postgres://, redis://:pw@,
    # https://user:token@), minus the placeholders documentation uses.
    "credential in URL": re.compile(
        r"\b[a-z][a-z0-9+.-]*://[^\s:/@]*:"
        r"(?!(?:password|pass|secret|root|test|!?change[-_]?me!?|x{3,}|\*+|\.{2,})@"
        r"|[<${%])"
        r"[^\s@/]{3,}@",
        re.IGNORECASE,
    ),
    # A credential key assigned a literal value. The key may carry a snake or
    # UPPER_SNAKE prefix (DB_PASSWORD=, MAIL_PASSWORD=, AWS_SECRET_ACCESS_KEY=),
    # which the old `\b` anchor missed; separators stay on one line.
    "assigned credential": re.compile(
        r"(?<![A-Za-z0-9])(?:[A-Za-z0-9]+_)*"
        r"(?:password|passwd|secret(?:_access)?(?:_key|_token)?|api[_-]?key"
        r"|access[_-]?token|auth[_-]?token)"
        r"[ \t]*[:=][ \t]*['\"`]?" + _NOT_A_LITERAL + r"[^\s'\"`]{4,}",
        re.IGNORECASE,
    ),
}
# Personal data that must not enter shared memory: Project Brain records are
# Git-tracked, and the Task Capsule repeats them into every prompt. The
# capsule gate used these first; the Brain write path applies them too.
PRIVATE_PATTERNS = {
    "email address": re.compile(r"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b", re.IGNORECASE),
    "phone number": re.compile(
        r"(?<!\w)(?:\+\d(?:[\d ().-]{6,}\d)|\(?\d{3}\)?[ .-]\d{3}[ .-]\d{4})(?!\w)"
    ),
    "customer identifier": re.compile(
        r"\b(?:(?:customer|patient)\s+(?:name|address|id)|"
        r"client\s+(?:name|address))\s*[:=]\s*\S+",
        re.IGNORECASE,
    ),
}


class AutomaticQueryError(ValueError):
    pass


def sanitize_automatic_query(text: str) -> str:
    """Remove recognized PII; secrets and copied conversation remain refused.

    Only automatic adapters use this. Direct queries and shared writes validate
    the original input. The result must be validated before it is persisted.
    """
    if any(pattern.search(text) for pattern in SECRET_PATTERNS.values()) or re.search(
        r"^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|stdout|stderr|log)\s*:",
        text, re.IGNORECASE | re.MULTILINE,
    ):
        raise AutomaticQueryError("Automatic memory query contains secret or raw data")
    original = text
    for label, pattern in PRIVATE_PATTERNS.items():
        if label == "customer identifier":
            # Labelled names/addresses may contain spaces, newlines and ';'.
            # Their end cannot be inferred safely: retain only the preceding
            # text, while the original request still reaches the native agent.
            match = pattern.search(text)
            if match:
                text = text[:match.start()]
        else:
            text = pattern.sub(" ", text)
    if text != original:
        # Labels/contact instructions left behind by redaction carry no topic.
        residue = re.findall(r"\w+", text.casefold())
        contact_words = frozenset("""
            a an and at by for from in is of on or the to with please thanks
            customer client patient user person email e mail address phone
            number mobile telephone tel contact call reach send name id identifier
            check fix review inspect look
            клиент клиента пациент пациента пользователь пользователя имя
            адрес почта телефон номер позвони позвонить связаться пожалуйста
        """.split())
        if not residue or all(word in contact_words for word in residue):
            return ""
    return text.strip(" \t\r\n,;")
