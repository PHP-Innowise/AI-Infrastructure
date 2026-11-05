//! Shared automatic-query, citation-path and secret-projection policy.
use crate::{Result, RuntimeError};
use fancy_regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    cell::OnceCell,
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

const LABELS: &[&str] = &[
    "private key",
    "GitHub token",
    "GitLab token",
    "Slack token",
    "Slack webhook",
    "Discord webhook",
    "Telegram bot token",
    "AWS access key",
    "Google API key",
    "Google OAuth token",
    "OpenAI-style token",
    "Stripe secret key",
    "npm token",
    "PyPI token",
    "Hugging Face token",
    "SendGrid key",
    "Shopify token",
    "DigitalOcean token",
    "JWT",
    "Laravel application key",
    "credential in URL",
    "authorization header",
    "basic credentials",
    "bearer token",
    "assigned credential",
    "email address",
    "phone number",
    "customer identifier",
    "customer number",
];
const WORD: &str = r"[\p{L}\p{N}_]";
const BOUNDARY: &str =
    r"(?:(?<![\p{L}\p{N}_])(?=[\p{L}\p{N}_])|(?<=[\p{L}\p{N}_])(?![\p{L}\p{N}_]))";
/// What an expression cannot match without: every group needs one of its
/// literals in the text's casefolded image, and `true` a numeric character.
type Needs = (&'static [&'static [&'static str]], bool);
const KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "apikey",
    "api_key",
    "api-key",
    "accesstoken",
    "accesskey",
    "access_token",
    "access_key",
    "access-token",
    "access-key",
    "authtoken",
    "auth_token",
    "auth-token",
    "refreshtoken",
    "refresh_token",
    "refresh-token",
    "privatekey",
    "private_key",
    "private-key",
    "accountkey",
    "account_key",
    "account-key",
    "sharedaccesskey",
    "sharedaccess_key",
    "sharedaccess-key",
    "shared_accesskey",
    "shared_access_key",
    "shared_access-key",
    "shared-accesskey",
    "shared-access_key",
    "shared-access-key",
];
/// Necessary literals for these exact sources and flags, pinned by SHA-256 (the
/// secret rows are the Python runtime's mask prefilters). A changed or unknown
/// expression keeps the full scan; this never decides what is secret.
const PINNED: &[(&str, &str, u64, Needs)] = &[
    (
        "private key",
        "42d5f36d732a5e75d1a8e481fda6c7a226948c56a005eb54e2ca65c3fd34156d",
        32,
        (&[&["-----begin "]], false),
    ),
    (
        "GitHub token",
        "68761294102b638abad4a26260391647743fa10bb874d45c6159f572cd96de7a",
        32,
        (
            &[&["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"]],
            false,
        ),
    ),
    (
        "GitLab token",
        "6cf7ed0fce6679d0e13564578f16dfece2fe3fc29729d876ceef111abf9f43eb",
        32,
        (&[&["gl"]], false),
    ),
    (
        "Slack token",
        "5ad38f6c78f86ce64f20a423704ec912647b1ec9efc40c7cf1599ba17ee007e1",
        32,
        (&[&["xox", "xapp-"]], false),
    ),
    (
        "Slack webhook",
        "f40b6113132ea8a516c11e934688b099d79e909cedcfa1184eb923bd6e7aceec",
        34,
        (&[&["hooks.slack.com/"]], false),
    ),
    (
        "Discord webhook",
        "b0d1d794dbbbf3d1a3d0dbee5e39bf5588ffac8c7da4f405ec37a5d27bfdfb12",
        34,
        (&[&[".com/api/webhooks/"]], false),
    ),
    (
        "Telegram bot token",
        "0d73b9142c78a8f6d9bd69c95f15a64a9e272ca7c6dcadb8ac5b3c425ce16959",
        32,
        (&[&[":aa"]], false),
    ),
    (
        "AWS access key",
        "7f8a678753a46ba3f6704958b8320cba8e038fd7fb8d8cb8e0c173db25ea24f9",
        32,
        (&[&["akia", "asia"]], false),
    ),
    (
        "Google API key",
        "34494ea7dde5044d930ce9fd926f1c23e7b36e5197c42007728ba26a1ae4dcd9",
        32,
        (&[&["aiza"]], false),
    ),
    (
        "Google OAuth token",
        "429afb0689fd198c3bf36fc2a1210aa9288b9010a916efa78f55cb4f39224c65",
        32,
        (&[&["ya29."]], false),
    ),
    (
        "OpenAI-style token",
        "1fcad04827f5b98cc5d41309369d1802d3639e23f4d35eddac1a93b52c009809",
        32,
        (&[&["sk-"]], false),
    ),
    (
        "Stripe secret key",
        "a6b6ed7945eefeb3695a7477b82f30c6c0b0abff91a14dd7a9f52e6cbb4c86ff",
        32,
        (&[&["rk_", "sk_"]], false),
    ),
    (
        "npm token",
        "3d97ff554ee799bff07ff452c2a5327654ec2fb76d7d92c1c625ab5318063983",
        32,
        (&[&["npm_"]], false),
    ),
    (
        "PyPI token",
        "f2edca40d579d9ccb9b07ae016b754d34086235f9903e03eff15fa2d586e964b",
        32,
        (&[&["pypi-age"]], false),
    ),
    (
        "Hugging Face token",
        "a0b78eeb650697addc2777dbf7554a95ebe8098be27b4e8622c467b9e22d8091",
        32,
        (&[&["hf_"]], false),
    ),
    (
        "SendGrid key",
        "3de13be719f8c8a1db81a233d44c9d75b3e9781564bc49fbff3d775b4b83f994",
        32,
        (&[&["sg."]], false),
    ),
    (
        "Shopify token",
        "aaea7614b9095bdafc9d0305256b1f029f42bc19cb97cc25b927f0bec50d78f9",
        32,
        (&[&["shp"]], false),
    ),
    (
        "DigitalOcean token",
        "5ef55bfd23ba33d17e5788cdc297e1744f2d4894b87d23053d28c17588ce0b6c",
        32,
        (&[&["_v1_"]], false),
    ),
    (
        "JWT",
        "d190c7a28bb6084cfc4bfafd2d67ff1a8781b83e38ce367267a5f4ecd2c5c875",
        32,
        (&[&["eyj"]], false),
    ),
    (
        "Laravel application key",
        "c291ae904226de167e2c62f924a58e32c809f577e0e7750021a68498773e3b9c",
        32,
        (&[&["app_key"]], false),
    ),
    (
        "credential in URL",
        "633cc88e42dc82836c82d03949215d6d65fbaff1d1cea0b8249131ddb793acc5",
        34,
        (&[&["://"]], false),
    ),
    (
        "authorization header",
        "a30545248891aef90bec6fbd535e17f6b391bc5fe49cb8e76cf5492e8ea48fb1",
        34,
        (&[&["authorization"]], false),
    ),
    (
        "basic credentials",
        "caf3189272465291e6592220028ef036076b261109ac5ee486603e0ca67a1cd8",
        34,
        (&[&["authorization"]], false),
    ),
    (
        "bearer token",
        "ee8f758f0d60a50dd2c85bb10a910c7b2fd8200fab3ec9b448b807d34afe3030",
        34,
        (&[&["bearer"]], false),
    ),
    (
        "assigned credential",
        "0f981eb49ea9ecedab5fa894792b7ddd1d5adb1821ab23d6b444ac27fa6922dd",
        34,
        (&[KEYS], false),
    ),
    (
        "email address",
        "f444518958b91a6122e99f67a0cb8c10bf1c01f8e06400a49b111cb6175d8874",
        34,
        (&[&["@"]], false),
    ),
    (
        "phone number",
        "fd79394deb01e0acd33d536dd5ae56debbcbb633a33d46cb71aea971160840a5",
        32,
        (&[], true),
    ),
    (
        "customer identifier",
        "2f6d76f83bb4a0e357b2348d3117a6ed41842405712b554dbed06fe6b8343c91",
        34,
        (&[&["customer", "patient", "client"]], false),
    ),
    (
        "customer number",
        "f5af53ce1aa1cecb81c9ccb58ab086335f3ea58200d6d386bb7f9b29e173e962",
        34,
        (
            &[&["customer", "patient", "клиент", "пациент", "client"]],
            true,
        ),
    ),
    (
        "RAW_TEXT_PATTERN",
        "1042ee0af7eac7f48266d3960874f2051546ec4928ad243de8d333818bda2917",
        42,
        (
            &[
                &[":"],
                &[
                    "user",
                    "assistant",
                    "system",
                    "developer",
                    "tool",
                    "prompt",
                    "response",
                    "reasoning",
                    "stdout",
                    "stderr",
                    "log",
                ],
            ],
            false,
        ),
    ),
    (
        "QUOTED_CREDENTIAL",
        "12b2aa54e3d46d139535f5984796b84cf848945b9510bf4c4ea95b617c58fb1c",
        42,
        (&[KEYS], false),
    ),
    (
        "PRIVATE_KEY_BLOCK",
        "6b20b58a656172adae553e591f45da556369cfa1eab9cbd8c652a804e8d2810d",
        48,
        (&[&["-----begin "]], false),
    ),
];
struct Rule {
    label: &'static str,
    original: String,
    flags: u64,
    needs: Option<&'static Needs>,
    expression: OnceLock<std::result::Result<Regex, String>>,
}
impl Rule {
    fn new(label: &'static str, original: String, flags: u64) -> Self {
        let digest = crate::domain::hex_lower(Sha256::digest(original.as_bytes()));
        let needs = PINNED
            .iter()
            .find(|row| row.0 == label && row.1 == digest && row.2 == flags)
            .map(|row| &row.3);
        Self {
            label,
            original,
            flags,
            needs,
            expression: OnceLock::new(),
        }
    }
    /// Compiled on first use: most texts carry no literal of most rules.
    fn regex(&self) -> Result<&Regex> {
        self.expression
            .get_or_init(|| compile(&self.original, self.flags))
            .as_ref()
            .map_err(|_| failure())
    }
}
/// One version of a text and its casefolded image, built on first use. Unicode
/// IGNORECASE also admits dotless/dotted i, long s and Kelvin; the image folds
/// them to ASCII. A replacement makes a new view, so no image outlives its text.
struct View<'a> {
    text: Cow<'a, str>,
    folded: OnceCell<String>,
}
impl<'a> View<'a> {
    fn new(text: impl Into<Cow<'a, str>>) -> Self {
        Self {
            text: text.into(),
            folded: OnceCell::new(),
        }
    }
    fn folded(&self) -> &str {
        self.folded.get_or_init(|| {
            if self.text.is_ascii() {
                self.text.to_ascii_lowercase()
            } else {
                crate::unicode_fold::fold(&self.text)
                    .replace('\u{131}', "i")
                    .replace('\u{307}', "")
            }
        })
    }
    /// Whether `rule` can match this text at all.
    fn admits(&self, rule: &Rule) -> bool {
        let Some((groups, digit)) = rule.needs else {
            return true;
        };
        (!digit || self.text.chars().any(char::is_numeric))
            && groups
                .iter()
                .all(|group| group.iter().any(|literal| self.folded().contains(literal)))
    }
}
struct Policy {
    secrets: Vec<Rule>,
    private: Vec<Rule>,
    raw: Rule,
    quoted: Rule,
    block: Rule,
    assigned_key: Rule,
    assigned_at: Rule,
    contacts: BTreeSet<String>,
    fingerprint: String,
}
fn failure() -> RuntimeError {
    RuntimeError::policy("Shared-text policy could not safely inspect this image")
}
fn translate(pattern: &str, flags: u64) -> String {
    let insensitive = flags & 2 != 0;
    let mut out = String::new();
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut at = 0;
    let mut in_class = false;
    while at < chars.len() {
        let c = chars[at];
        if c == '\\' && at + 1 < chars.len() {
            let next = chars[at + 1];
            match next {
                'w' => out.push_str(if in_class { r"\p{L}\p{N}_" } else { WORD }),
                'W' => out.push_str(r"[^\p{L}\p{N}_]"),
                'b' if !in_class => out.push_str(BOUNDARY),
                's' => out.push_str(if in_class {
                    r"\s\u{1c}-\u{1f}"
                } else {
                    r"[\s\u{1c}-\u{1f}]"
                }),
                'Z' => out.push_str(r"\z"),
                _ => {
                    out.push('\\');
                    out.push(next);
                }
            }
            at += 2;
            continue;
        }
        if c == '[' {
            in_class = true;
            out.push(c);
            at += 1;
            continue;
        }
        if c == ']' {
            in_class = false;
            out.push(c);
            at += 1;
            continue;
        }
        if insensitive
            && in_class
            && at + 2 < chars.len()
            && chars[at + 1] == '-'
            && ((c == 'A' && chars[at + 2] == 'Z') || (c == 'a' && chars[at + 2] == 'z'))
        {
            out.push(c);
            out.push('-');
            out.push(chars[at + 2]);
            out.push_str("İıſK");
            at += 3;
            continue;
        }
        // Python IGNORECASE additionally folds the dotted/dotless I; spans remain in the original UTF-8 image.
        if insensitive && !in_class && (c == 'i' || c == 'I') {
            out.push_str("[iIİı]");
        } else {
            out.push(c);
        }
        at += 1;
    }
    let mut modes = String::new();
    if flags & 2 != 0 {
        modes.push('i');
    }
    if flags & 8 != 0 {
        modes.push('m');
    }
    if flags & 16 != 0 {
        modes.push('s');
    }
    if modes.is_empty() {
        out
    } else {
        format!("(?{modes}){out}")
    }
}
fn compile(pattern: &str, flags: u64) -> std::result::Result<Regex, String> {
    Regex::new(&translate(pattern, flags))
        .map_err(|_| "Shared-text policy expression is invalid".into())
}
fn rules(value: &Value) -> std::result::Result<Vec<Rule>, String> {
    let mut out = Vec::new();
    for (label, row) in value.as_object().ok_or("Policy rules must be an object")? {
        let label = LABELS
            .iter()
            .copied()
            .find(|known| *known == label)
            .ok_or("Unknown policy label")?;
        let original = row["pattern"]
            .as_str()
            .ok_or("Policy pattern missing")?
            .to_string();
        let flags = row["flags"].as_u64().ok_or("Policy flags missing")?;
        out.push(Rule::new(label, original, flags));
    }
    Ok(out)
}
fn initialize() -> std::result::Result<Policy, String> {
    let source: Value = serde_json::from_str(include_str!("privacy-policy.json"))
        .map_err(|_| "Invalid policy definition")?;
    let secrets = rules(&source["SECRET_PATTERNS"])?;
    let private = rules(&source["PRIVATE_PATTERNS"])?;
    let make = |key: &'static str| -> std::result::Result<Rule, String> {
        Ok(Rule::new(
            key,
            source[key]["pattern"]
                .as_str()
                .ok_or("Missing policy pattern")?
                .into(),
            source[key]["flags"].as_u64().ok_or("Missing flags")?,
        ))
    };
    let assigned = secrets
        .iter()
        .find(|rule| rule.label == "assigned credential")
        .ok_or("Missing assignment policy")?;
    let assigned_at = Rule::new(
        "assigned credential anchor",
        format!(r"\A(?:{})", assigned.original),
        assigned.flags,
    );
    let assigned_key = Rule::new(
        "assigned credential key",
        r"(?<![A-Za-z0-9])(?:password|passwd|secret(?:_access)?(?:_key|_token)?|api[_-]?key|access[_-]?(?:token|key)|auth[_-]?token|refresh[_-]?token|private[_-]?key|account[_-]?key|shared[_-]?access[_-]?key)[ \t]*[:=]".into(),
        34,
    );
    let mut parts = vec![
        "1".to_string(),
        source["PRIVATE_KEY_BLOCK"]["pattern"]
            .as_str()
            .ok_or("Policy block missing")?
            .into(),
        source["QUOTED_CREDENTIAL"]["pattern"]
            .as_str()
            .ok_or("Quoted policy missing")?
            .into(),
    ];
    let mut keyed = vec![
        "assigned credential",
        "Laravel application key",
        "authorization header",
        "basic credentials",
    ];
    keyed.sort();
    parts.extend(keyed.into_iter().map(String::from));
    for rule in &secrets {
        parts.extend([
            rule.label.to_string(),
            rule.original.clone(),
            rule.flags.to_string(),
        ]);
    }
    let fingerprint = crate::domain::hex_lower(Sha256::digest(parts.join("\0").as_bytes()));
    let contacts = source["CONTACT_WORDS"]
        .as_array()
        .ok_or("Contact words missing")?
        .iter()
        .map(|word| {
            word.as_str()
                .map(String::from)
                .ok_or("Contact word invalid")
        })
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    Ok(Policy {
        secrets,
        private,
        raw: make("RAW_TEXT_PATTERN")?,
        quoted: make("QUOTED_CREDENTIAL")?,
        block: make("PRIVATE_KEY_BLOCK")?,
        assigned_key,
        assigned_at,
        contacts,
        fingerprint,
    })
}
fn policy() -> Result<&'static Policy> {
    static POLICY: OnceLock<std::result::Result<Policy, String>> = OnceLock::new();
    POLICY
        .get_or_init(initialize)
        .as_ref()
        .map_err(|_| failure())
}
/// Expressions applied through `policy_pattern_matches`, by source and flags.
type Compiled = Mutex<HashMap<(String, u64), Arc<Regex>>>;
/// Apply a static shared policy expression with Python Unicode boundaries.
pub fn policy_pattern_matches(pattern: &str, flags: u64, text: &str) -> Result<bool> {
    // Callers pass a fixed set of expressions; each one compiles once.
    static COMPILED: OnceLock<Compiled> = OnceLock::new();
    let expression = {
        let mut compiled = COMPILED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match compiled.get(&(pattern.to_string(), flags)) {
            Some(expression) => expression.clone(),
            None => {
                let expression = Arc::new(compile(pattern, flags).map_err(|_| failure())?);
                compiled.insert((pattern.to_string(), flags), expression.clone());
                expression
            }
        }
    };
    expression.is_match(text).map_err(|_| failure())
}
fn match_ranges(rule: &Rule, view: &View, p: &Policy) -> Result<Vec<(usize, usize)>> {
    if !view.admits(rule) {
        return Ok(Vec::new());
    }
    let text: &str = &view.text;
    if rule.label != "assigned credential" {
        return rule
            .regex()?
            .find_iter(text)
            .map(|found| found.map(|m| (m.start(), m.end())).map_err(|_| failure()))
            .collect();
    }
    let mut result = Vec::new();
    let mut resume = 0;
    for candidate in p.assigned_key.regex()?.find_iter(text) {
        let candidate = candidate.map_err(|_| failure())?;
        let mut start = candidate.start();
        if start < resume {
            continue;
        }
        while start > resume && text[..start].ends_with('_') {
            let segment_end = start - 1;
            let mut segment_start = segment_end;
            while segment_start > resume {
                let Some((position, c)) = text[..segment_start].char_indices().next_back() else {
                    break;
                };
                if !assigned_prefix(c) {
                    break;
                }
                segment_start = position;
            }
            if segment_start == segment_end {
                break;
            }
            if segment_start == resume
                && resume > 0
                && text[..resume].chars().last().is_some_and(assigned_prefix)
            {
                break;
            }
            start = segment_start;
        }
        if let Some(found) = p
            .assigned_at
            .regex()?
            .find(&text[start..])
            .map_err(|_| failure())?
        {
            let span = (start + found.start(), start + found.end());
            resume = span.1;
            result.push(span);
        }
    }
    Ok(result)
}
fn assigned_prefix(c: char) -> bool {
    c.is_ascii_alphanumeric() || ['İ', 'ı', 'ſ', 'K'].contains(&c)
}
fn first_secret(view: &View, p: &Policy) -> Result<Option<&'static str>> {
    for rule in &p.secrets {
        if !match_ranges(rule, view, p)?.is_empty() {
            return Ok(Some(rule.label));
        }
    }
    Ok(None)
}
pub fn secret_label(text: &str) -> Result<Option<&'static str>> {
    first_secret(&View::new(text), policy()?)
}
/// What sensitive data `text` appears to carry, worded as the retired
/// runtime named it in a refusal: "a possible <secret>" or "personal data
/// (<kind>)". Never the matched value.
pub fn sensitive_label(text: &str) -> Result<Option<String>> {
    let p = policy()?;
    let view = View::new(text);
    if let Some(label) = first_secret(&view, p)? {
        return Ok(Some(format!("a possible {label}")));
    }
    for rule in &p.private {
        if view.admits(rule) && rule.regex()?.is_match(text).map_err(|_| failure())? {
            return Ok(Some(format!("personal data ({})", rule.label)));
        }
    }
    Ok(None)
}
pub fn secret_policy_fingerprint() -> Result<String> {
    Ok(policy()?.fingerprint.clone())
}
pub fn guard_shared_text(label: &str, values: &[String]) -> Result<()> {
    if let Some(kind) = sensitive_label(&values.join("\n"))? {
        return Err(RuntimeError::policy(format!(
            "{label} contains {kind}; replace it with a sanitized summary - nothing was stored"
        )));
    }
    Ok(())
}
fn replace_ranges(text: &str, ranges: &[(usize, usize, String)]) -> String {
    let mut out = String::new();
    let mut end = 0;
    for (start, stop, replacement) in ranges {
        if *start < end {
            end = end.max(*stop);
            continue;
        }
        out.push_str(&text[end..*start]);
        out.push_str(replacement);
        end = *stop;
    }
    out.push_str(&text[end..]);
    out
}
fn masked_span(
    p: &Policy,
    text: &str,
    label: &str,
    start: usize,
    end: usize,
) -> Result<(usize, usize)> {
    if label == "private key" {
        return Ok((
            start,
            p.block
                .regex()?
                .find(&text[start..])
                .map_err(|_| failure())?
                .filter(|m| m.start() == 0)
                .map(|m| start + m.end())
                .unwrap_or(end),
        ));
    }
    if ![
        "assigned credential",
        "Laravel application key",
        "authorization header",
        "basic credentials",
    ]
    .contains(&label)
    {
        return Ok((start, end));
    }
    let Some((position, c)) = text[start..end]
        .char_indices()
        .find(|(_, c)| *c == ':' || *c == '=')
    else {
        return Ok((start, end));
    };
    let mut value = start + position + c.len_utf8();
    while value < end && matches!(text.as_bytes()[value], b' ' | b'\t') {
        value += 1;
    }
    if label == "assigned credential"
        && let Some(capture) = p
            .quoted
            .regex()?
            .captures(&text[start..])
            .map_err(|_| failure())?
        && capture.get(0).is_some_and(|m| m.start() == 0)
        && let (Some(quote), Some(whole)) = (capture.get(1), capture.get(0))
    {
        return Ok((start + quote.start(), start + whole.end()));
    }
    if text[end..]
        .chars()
        .next()
        .is_some_and(|c| ['\'', '"', '\u{60}'].contains(&c))
    {
        return Ok((value, end));
    }
    Ok((
        value,
        text[end..]
            .find('\n')
            .map(|offset| end + offset)
            .unwrap_or(text.len()),
    ))
}
pub fn mask_secrets(original: &str) -> Result<(String, usize)> {
    let p = policy()?;
    let mut view = View::new(original);
    let mut count = 0;
    for _ in 0..3 {
        let text: &str = &view.text;
        let mut spans = Vec::new();
        for rule in &p.secrets {
            for (start, end) in match_ranges(rule, &view, p)? {
                let (a, b) = masked_span(p, text, rule.label, start, end)?;
                spans.push((a, b, rule.label));
            }
        }
        spans.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
        if spans.is_empty() {
            return Ok((view.text.into_owned(), count));
        }
        let mut ranges = Vec::new();
        let mut end = 0;
        for (start, stop, label) in spans {
            if start < end {
                if let Some(last) = ranges.last_mut() {
                    let row: &mut (usize, usize, String) = last;
                    row.1 = row.1.max(stop);
                }
                end = end.max(stop);
                continue;
            }
            ranges.push((start, stop, format!("[redacted: {label}]")));
            end = stop;
            count += 1;
        }
        view = View::new(replace_ranges(text, &ranges));
    }
    if first_secret(&view, p)?.is_some() {
        return Err(RuntimeError::policy(
            "possible secret survives masking; value intentionally not printed",
        ));
    }
    Ok((view.text.into_owned(), count))
}
pub fn indexed_secret_policy_reason(texts: &[&str]) -> Option<&'static str> {
    for text in texts {
        if match mask_secrets(text) {
            Ok((masked, _)) => masked != *text,
            Err(_) => true,
        } {
            return Some("secret-policy");
        }
    }
    None
}
/// `view` with every match of `rule` replaced by a space, kept after the label
/// before a `value` group when `keep_label`.
fn substitute<'a>(rule: &Rule, view: View<'a>, keep_label: bool) -> Result<View<'a>> {
    if !view.admits(rule) {
        return Ok(view);
    }
    let text: &str = &view.text;
    let mut ranges = Vec::new();
    for found in rule.regex()?.captures_iter(text) {
        let found = found.map_err(|_| failure())?;
        let whole = found.get(0).ok_or_else(failure)?;
        let replacement = if keep_label {
            found
                .name("value")
                .map(|value| format!("{} ", &text[whole.start()..value.start()]))
                .unwrap_or(" ".into())
        } else {
            " ".into()
        };
        ranges.push((whole.start(), whole.end(), replacement));
    }
    if ranges.is_empty() {
        return Ok(view);
    }
    Ok(View::new(replace_ranges(text, &ranges)))
}
pub fn sanitize_automatic_query(original: &str) -> Result<String> {
    let p = policy()?;
    let mut view = substitute(&p.block, View::new(original), false)?;
    view = substitute(&p.quoted, view, false)?;
    let identifier = p
        .private
        .iter()
        .find(|rule| rule.label == "customer identifier")
        .ok_or_else(failure)?;
    if view.admits(identifier)
        && let Some(found) = identifier
            .regex()?
            .find(&view.text)
            .map_err(|_| failure())?
    {
        view = View::new(view.text[..found.start()].to_string());
    }
    for _ in 0..3 {
        let previous = view.text.clone();
        for rule in &p.secrets {
            let ranges = match_ranges(rule, &view, p)?
                .into_iter()
                .map(|(a, b)| (a, b, " ".into()))
                .collect::<Vec<_>>();
            if !ranges.is_empty() {
                view = View::new(replace_ranges(&view.text, &ranges));
            }
        }
        for rule in &p.private {
            if rule.label != "customer identifier" {
                view = substitute(rule, view, true)?;
            }
        }
        view = substitute(&p.raw, view, false)?;
        if view.text == previous {
            break;
        }
    }
    if first_secret(&view, p)?.is_some() {
        return Err(RuntimeError::policy(
            "Automatic memory query still carries a secret after redaction",
        ));
    }
    let text = view.text;
    if text != original {
        let folded = crate::query::folded(&text);
        let residue = folded
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>();
        if residue.is_empty() || residue.iter().all(|word| p.contacts.contains(*word)) {
            return Ok(String::new());
        }
    }
    Ok(text.trim_matches([' ', '\t', '\r', '\n', ',', ';']).into())
}
pub fn source_path_problem(value: &str) -> Option<&'static str> {
    let head = value.split('#').next().unwrap_or(value);
    let parts = head
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let name = crate::query::folded(parts.last().copied().unwrap_or(""));
    let suffix = name
        .rsplit_once('.')
        .map(|(_, suffix)| suffix)
        .unwrap_or("");
    if name == ".env"
        || (name.starts_with(".env.") && name.len() > 5)
        || ["secret", "secrets"].contains(&name.as_str())
        || (name.starts_with("id_")
            && name.len() > 3
            && name[3..].chars().all(|c| c.is_ascii_alphanumeric()))
        || ["pem", "key", "p12", "pfx", "jks", "keystore"].contains(&suffix)
    {
        return Some("Sensitive source paths are refused");
    }
    if [
        "memory-bank/local/",
        "memory-bank/chunks/",
        "project-brain/local/",
        "project-brain/dynamic/",
        "project-brain/archive/",
        "project-brain/control/",
    ]
    .iter()
    .any(|prefix| head.starts_with(prefix))
    {
        return Some("Cite canonical project sources, not private runtime state or derived memory");
    }
    if parts.iter().any(|part| {
        let value = crate::query::folded(part);
        [
            ".git",
            ".ssh",
            ".aws",
            ".kube",
            "node_modules",
            "vendor",
            ".venv",
            "__pycache__",
            "secrets",
            ".secrets",
            "credentials",
        ]
        .contains(&value.as_str())
            || value.starts_with(".env")
    }) || ["pem", "key", "p12", "pfx", "sqlite", "db"].contains(&suffix)
        || name == "credentials.json"
    {
        return Some("Private or dependency source paths are refused");
    }
    if head.starts_with('/') || parts.contains(&"..") || head.contains('\\') || head.contains(':') {
        return Some("Use project-relative source paths");
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;

    fn pinned(p: &Policy) -> impl Iterator<Item = &Rule> {
        p.secrets
            .iter()
            .chain(&p.private)
            .chain([&p.raw, &p.quoted, &p.block])
    }
    /// The case and Unicode variants Python IGNORECASE admits, at random.
    fn variant(sample: &str, seed: &mut u64) -> String {
        sample
            .chars()
            .map(|c| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 7;
                *seed ^= *seed << 17;
                let roll = *seed % 9;
                match (c, roll) {
                    ('s' | 'S', 0) => 'ſ',
                    ('k' | 'K', 0) => '\u{212a}',
                    ('i' | 'I', 0) => 'İ',
                    ('i' | 'I', 1) => 'ı',
                    ('т' | 'Т', 0) => '\u{1c84}',
                    ('1', 0) => '\u{661}',
                    (_, 0..=3) if c.is_lowercase() => c.to_uppercase().next().unwrap_or(c),
                    (_, 0..=3) => c.to_lowercase().next().unwrap_or(c),
                    _ => c,
                }
            })
            .collect()
    }

    #[test]
    fn every_expression_compiles_and_keeps_its_pinned_literals() {
        let p = policy().unwrap();
        for rule in pinned(p).chain([&p.assigned_key, &p.assigned_at]) {
            assert!(rule.regex().is_ok(), "{}", rule.label);
        }
        for rule in pinned(p) {
            assert!(
                rule.needs.is_some(),
                "{} lost its pinned literals",
                rule.label
            );
        }
    }

    #[test]
    fn pinned_literals_never_skip_a_match() {
        // Secret scanners refuse a push that holds these vendor shapes, so the
        // samples are joined at run time; the policy sees the same text.
        let sendgrid = [
            "SG",
            ".abcdefghijklmnopqrstuv.abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG",
        ]
        .concat();
        let vendors = [
            "shpat",
            "_0123456789abcdef0123456789abcdef dop",
            "_v1_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ]
        .concat();
        let samples = [
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY-----",
            "use ghp_aB3dE5fG7hI9jK1lM3nO5pQ7rS9tU1vW3xY5 or github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "glpat-abcdefghij0123456789 and xoxb-1234567890-abcdefghij and xapp-1-A0123456789-abc",
            "https://hooks.slack.com/services/T0000/B0000/XXXXXXXXXXXXXXXXXXXXXXXX",
            "https://discordapp.com/api/webhooks/123456/abcdefghijklmnopqrstuvwxyz0123456789",
            "bot 123456789:AAabcdefghijklmnopqrstuvwxyz0123456",
            "AKIAIOSFODNN7EXAMPLE ASIAIOSFODNN7EXAMPLE",
            "AIzaSyA1234567890abcdefghijklmnopqrstuv ya29.a0AfH6SMBx1234567890abcdefgh",
            "sk-proj-abcdefghijklmnopqrstuvwxyz012345 sk_live_abcdefghijklmnop1234 rk_test_abcdefghijklmnop1234",
            "npm_abcdefghijklmnopqrstuvwxyz0123456789 hf_abcdefghijklmnopqrstuvwxyz01234567",
            "pypi-AgEIcHlwaS5vcmcCJDAwMDAwMDAwLTAwMDAtMDAwMC0wMDAwLTAwMDAwMDAwMDAwMAACKlsz",
            sendgrid.as_str(),
            vendors.as_str(),
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "APP_KEY=base64:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG=",
            "postgres://admin:hunter2pass@db.example.com/app",
            "Authorization: Bearer abcdef1234567890abcdef1234567890\nProxy-Authorization: Basic dXNlcjpwYXNzd29yZDEyMzQ=",
            "curl -H 'bearer abcdef1234567890abcdef1234567890'",
            "DB_PASSWORD=Sup3rS3cretValue\napi_key: 'abc123def456'\nshared-access-key = xyz789abc\nAuth-Token: q1w2e3r4t5",
            "secret_access_key=\"wJalrXUtnFEMI/K7MDENG\" private_key=AbC123dEf",
            "write to john.doe@example.com or call +1 (555) 123-4567 or 555-123-4567",
            "Customer name: John Smith\nclient address = Main Street 1\npatient id: 77",
            "customer id 12345, patient #123456, client no. 4321, клиент номер 98765, номер клиента 4567, пациентка № 123",
            "user: hello\nassistant: hi\n  stderr : boom\nLOG:done",
        ];
        let p = policy().unwrap();
        let mut seen = BTreeSet::new();
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        for sample in samples {
            for round in 0..200 {
                let text = if round == 0 {
                    sample.to_string()
                } else {
                    variant(sample, &mut seed)
                };
                let view = View::new(text.as_str());
                for rule in pinned(p) {
                    if rule.regex().unwrap().is_match(&text).unwrap() {
                        seen.insert(rule.label);
                        assert!(view.admits(rule), "{} skipped {text:?}", rule.label);
                    }
                }
            }
        }
        // Every pinned rule was exercised by at least one sample.
        assert_eq!(seen.len(), pinned(p).count(), "{seen:?}");
    }
}
