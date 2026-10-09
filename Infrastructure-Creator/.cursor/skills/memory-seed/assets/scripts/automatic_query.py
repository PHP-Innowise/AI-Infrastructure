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


# Transcript role prefixes ("user:", "stderr:") mark pasted conversation and
# logs. The prefix goes; the words after it are often exactly the error text
# the request is about.
RAW_TEXT_PATTERN = re.compile(
    r"^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|"
    r"stdout|stderr|log)\s*:",
    re.IGNORECASE | re.MULTILINE,
)
# A credential assigned a quoted value: the whole literal goes, to its closing
# quote or, when the quote never closes, to the end of the line. The detection
# pattern above stops at the first space - enough to refuse a write, but it
# left "beta gamma'" of `password='alpha beta gamma'` in an automatic query,
# and from there in a task goal and a manifest.
QUOTED_CREDENTIAL = re.compile(
    r"(?<![A-Za-z0-9])(?:[A-Za-z0-9]+_)*"
    r"(?:password|passwd|secret(?:_access)?(?:_key|_token)?|api[_-]?key"
    r"|access[_-]?token|auth[_-]?token)"
    r"[ \t]*[:=][ \t]*(['\"`])[^\n]*?(?:\1|$)",
    re.IGNORECASE | re.MULTILINE,
)
# SECRET_PATTERNS recognises a private key by its header line alone; the body
# under it is the secret, so the whole block goes, to its footer or the end.
PRIVATE_KEY_BLOCK = re.compile(
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|\Z)",
    re.DOTALL,
)
# Words that only framed removed data: when nothing else is left, the request
# was about a person, not about the project, and there is nothing to search.
CONTACT_WORDS = frozenset("""
    a an and at by for from in is of on or the to with please thanks
    customer client patient user person email e mail address phone
    number mobile telephone tel contact call reach send name id identifier
    check fix review inspect look
    клиент клиента пациент пациента пользователь пользователя имя
    адрес почта телефон номер позвони позвонить связаться пожалуйста
""".split())


class AutomaticQueryError(ValueError):
    pass


def sanitize_automatic_query(text: str) -> str:
    """The request as an automatic adapter may search with it.

    Secrets, personal data and transcript role prefixes are cut out and the
    words around them stay: prompts arrive as people write them, with pasted
    logs and addresses, and refusing the whole prompt cost 35 of 60 first
    prompts on one real project their memory. A labelled name or address has
    no reliable end - it may hold spaces, newlines and ';' - so the text from
    its label on goes. When what is left only framed the removed data, the
    result is empty: there is nothing to search, and no fallback to a task
    goal or branch name should stand in for it.

    Only automatic adapters (hooks, Harness) use this. Direct queries and
    shared writes still validate, and refuse, the original input.
    """
    original = text
    text = PRIVATE_KEY_BLOCK.sub(" ", text)
    text = QUOTED_CREDENTIAL.sub(" ", text)
    labelled = PRIVATE_PATTERNS["customer identifier"].search(text)
    if labelled:
        text = text[:labelled.start()]
    for _ in range(3):
        before = text
        for pattern in SECRET_PATTERNS.values():
            text = pattern.sub(" ", text)
        for label, pattern in PRIVATE_PATTERNS.items():
            if label != "customer identifier":
                text = pattern.sub(" ", text)
        text = RAW_TEXT_PATTERN.sub(" ", text)
        if text == before:
            break
    if any(pattern.search(text) for pattern in SECRET_PATTERNS.values()):
        # Redaction that does not converge is not trusted with the rest.
        raise AutomaticQueryError("Automatic memory query still carries a secret after redaction")
    if text != original:
        residue = re.findall(r"\w+", text.casefold())
        if not residue or all(word in CONTACT_WORDS for word in residue):
            return ""
    return text.strip(" \t\r\n,;")
