//! Display-only shell check classification. Never executes a command.
use serde_json::{json, Value as V};
#[derive(Clone)]
struct Word {
    raw: String,
    value: String,
}
struct Part {
    text: String,
    next: Option<&'static str>,
}
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn is_word_boundary_char(c: char) -> bool {
    !c.is_ascii_alphanumeric() && c != '_'
}
fn starts_with_keyword(s: &str, keyword: &str, require_space: bool) -> bool {
    let Some(rest) = s.strip_prefix(keyword) else {
        return false;
    };
    if require_space {
        rest.chars().next().is_some_and(char::is_whitespace)
    } else {
        rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace)
    }
}
fn starts_with_word_boundary(s: &str, keyword: &str) -> bool {
    let Some(rest) = s.strip_prefix(keyword) else {
        return false;
    };
    rest.chars().next().is_none_or(is_word_boundary_char)
}
fn contains_word(s: &str, word: &str) -> bool {
    s.match_indices(word).any(|(idx, _)| {
        let before_ok = s[..idx]
            .chars()
            .next_back()
            .is_none_or(is_word_boundary_char);
        let after_ok = s[idx + word.len()..]
            .chars()
            .next()
            .is_none_or(is_word_boundary_char);
        before_ok && after_ok
    })
}
fn is_php_executable(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("php") else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    if let Some((major, minor)) = rest.split_once('.') {
        !major.is_empty()
            && major.bytes().all(|b| b.is_ascii_digit())
            && !minor.is_empty()
            && minor.bytes().all(|b| b.is_ascii_digit())
    } else {
        rest.bytes().all(|b| b.is_ascii_digit())
    }
}
fn is_env_assignment_prefix(s: &str) -> bool {
    let Some((name, _)) = s.split_once('=') else {
        return false;
    };
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && is_ident_start(bytes[0])
        && bytes[1..].iter().copied().all(is_ident_continue)
}
fn is_pure_env_assignments(s: &str) -> bool {
    let trimmed = s.trim();
    !trimmed.is_empty() && trimmed.split_whitespace().all(is_env_assignment_prefix)
}
fn is_redirect_token(raw: &str) -> bool {
    if raw == "<" {
        return true;
    }
    let rest = raw.trim_start_matches(|c: char| c.is_ascii_digit());
    let Some(after_gt) = rest.strip_prefix(">>").or_else(|| rest.strip_prefix('>')) else {
        return false;
    };
    if after_gt.is_empty() {
        return true;
    }
    if let Some(fd) = after_gt.strip_prefix('&') {
        fd.len() == 1 && fd.as_bytes()[0].is_ascii_digit()
    } else {
        !after_gt.chars().any(char::is_whitespace)
    }
}
fn redirect_takes_next(raw: &str) -> bool {
    if raw == "<" {
        return true;
    }
    let rest = raw.trim_start_matches(|c: char| c.is_ascii_digit());
    rest == ">" || rest == ">>"
}
fn is_exit_status_var(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("exit $") else {
        return false;
    };
    if let Some(after) = rest.strip_prefix('?') {
        after.is_empty() || after.chars().next().is_some_and(char::is_whitespace)
    } else if let Some(after) = rest.strip_prefix("LASTEXITCODE") {
        after.chars().next().is_none_or(is_word_boundary_char)
    } else {
        false
    }
}
fn words(text: &str) -> Vec<Word> {
    let mut out = vec![];
    let chars = text.chars().collect::<Vec<_>>();
    let (mut raw, mut value, mut quote, mut open) = (String::new(), String::new(), None, false);
    let mut at = 0;
    while at <= chars.len() {
        let ch = chars.get(at).copied();
        if ch.is_none() || quote.is_none() && ch.is_some_and(char::is_whitespace) {
            if open {
                out.push(Word {
                    raw: std::mem::take(&mut raw),
                    value: std::mem::take(&mut value),
                });
            }
            open = false;
            at += 1;
            continue;
        }
        let ch = ch.expect("present char");
        open = true;
        raw.push(ch);
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' && chars.get(at + 1) == Some(&'"') {
                at += 1;
                raw.push('"');
                value.push('"');
            } else {
                value.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else {
            value.push(ch);
        }
        at += 1;
    }
    out
}
fn parts(text: &str) -> Vec<Part> {
    let chars = text.chars().collect::<Vec<_>>();
    let (mut out, mut current, mut quote) = (vec![], String::new(), None);
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if let Some(q) = quote {
            current.push(c);
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' && chars.get(at + 1) == Some(&'"') {
                at += 1;
                current.push('"');
            }
            at += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            current.push(c);
            at += 1;
            continue;
        }
        let separator = match (c, chars.get(at + 1).copied()) {
            ('&', Some('&')) => Some("&&"),
            ('|', Some('|')) => Some("||"),
            ('|', _) => Some("|"),
            (';', _) => Some(";"),
            ('&', _)
                if chars.get(at.wrapping_sub(1)) != Some(&'>')
                    && chars.get(at + 1) != Some(&'>') =>
            {
                Some("&")
            }
            _ => None,
        };
        if let Some(next) = separator {
            if next != "&" || !current.trim().is_empty() {
                out.push(Part {
                    text: current.trim().into(),
                    next: Some(next),
                });
                current.clear();
            }
            at += if next.len() == 2 { 2 } else { 1 };
            continue;
        }
        current.push(c);
        at += 1;
    }
    out.push(Part {
        text: current.trim().into(),
        next: None,
    });
    out.into_iter()
        .filter(|p| !p.text.is_empty() || p.next.is_none())
        .collect()
}
fn exe(word: &Word) -> String {
    let name = word
        .value
        .replace('\\', "/")
        .trim_start_matches("./")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    [".bat", ".cmd", ".exe", ".phar"]
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix).map(str::to_owned))
        .unwrap_or(name)
}
fn significant(args: &[Word], family: &str) -> String {
    let flags = [
        "-c",
        "--configuration",
        "--memory-limit",
        "--error-format",
        "--standard",
        "--report",
        "--format",
        "--bootstrap",
        "--log-junit",
        "--cache-dir",
        "--config",
        "--set",
        "--autoload-file",
        "--threads",
        "-j",
        "--processes",
        "-p",
    ];
    let mut out = vec![];
    let mut at = 0;
    while at < args.len() {
        let arg = &args[at].value;
        let parsed = arg
            .strip_prefix("--")
            .and_then(|v| {
                let (k, v) = v
                    .split_once('=')
                    .map(|(k, v)| (k, Some(v)))
                    .unwrap_or((v, None));
                if ["filter", "testsuite", "group", "exclude-group", "level"].contains(&k) {
                    Some((k, v))
                } else {
                    None
                }
            })
            .or_else(|| {
                if family == "PHPStan" && arg == "-l" {
                    Some(("level", None))
                } else {
                    None
                }
            });
        if let Some((key, value)) = parsed {
            let value = if let Some(v) = value {
                v
            } else {
                at += 1;
                args.get(at).map(|w| w.value.as_str()).unwrap_or("")
            };
            out.push(format!("--{key}={value}"));
            at += 1;
            continue;
        }
        if arg.starts_with('-') {
            if flags.contains(&arg.as_str()) {
                at += 1;
            }
        } else {
            out.push(arg.replace('\\', "/"));
        }
        at += 1;
    }
    out.join(" ")
}
#[derive(Clone)]
struct Found {
    family: &'static str,
    target: String,
    fixer: bool,
}
fn found(family: &'static str, target: String, default: &str, fixer: bool) -> Option<Found> {
    Some(Found {
        family,
        target: if target.is_empty() {
            default.into()
        } else {
            target
        },
        fixer,
    })
}
fn identify(list: &[Word]) -> Option<Found> {
    let mut i = 0;
    let name = |i: usize| list.get(i).map(exe).unwrap_or_default();
    if is_php_executable(&name(0)) {
        if list.get(1).is_some_and(|w| w.value == "-l") && list.len() > 2 {
            return found("PHP lint", list[2].value.replace('\\', "/"), "", false);
        }
        i += 1;
        while list.get(i).is_some_and(|w| w.value.starts_with('-')) {
            i += if list
                .get(i)
                .is_some_and(|w| w.value == "-d" || w.value == "-c")
            {
                2
            } else {
                1
            };
        }
    }
    let program = name(i);
    let sub = list.get(i + 1).map(|w| w.value.as_str()).unwrap_or("");
    let args = list.get(i + 1..).unwrap_or(&[]);
    if program == "artisan" {
        return if sub == "test" {
            found(
                "Artisan test",
                significant(list.get(i + 2..).unwrap_or(&[]), ""),
                "all tests",
                false,
            )
        } else {
            None
        };
    }
    if program == "composer" {
        if sub == "exec" {
            let skip = if list.get(i + 2).is_some_and(|w| w.value == "--") {
                i + 3
            } else {
                i + 2
            };
            return identify(list.get(skip..).unwrap_or(&[]));
        }
        if ["validate", "audit"].contains(&sub) {
            return found("Composer", sub.into(), "", false);
        }
        if sub == "test"
            || ["run", "run-script"].contains(&sub)
                && list.get(i + 2).is_some_and(|w| w.value == "test")
        {
            return found("Composer", "test".into(), "", false);
        }
        return None;
    }
    if program == "console" && sub.starts_with("lint:") && sub.len() > 5 {
        return found(
            "Symfony lint",
            format!(
                "{sub} {}",
                significant(list.get(i + 2..).unwrap_or(&[]), "")
            )
            .trim()
            .into(),
            "",
            false,
        );
    }
    if program == "console" && sub == "doctrine:schema:validate" {
        return found("Doctrine schema", "validate".into(), "", false);
    }
    if program == "wp"
        && ["core", "plugin"].contains(&sub)
        && list
            .get(i + 2)
            .is_some_and(|w| w.value == "verify-checksums")
    {
        return found("WP checksums", sub.into(), "", false);
    }
    if program == "codecept" && sub == "run" {
        return found(
            "Codeception",
            significant(list.get(i + 2..).unwrap_or(&[]), ""),
            "all tests",
            false,
        );
    }
    if args
        .iter()
        .any(|w| ["--version", "--help", "-V", "-h"].contains(&w.value.as_str()))
    {
        return None;
    }
    let (family, scope) = match program.as_str() {
        "phpunit" | "simple-phpunit" => ("PHPUnit", "tests"),
        "pest" => ("Pest", "tests"),
        "paratest" => ("ParaTest", "tests"),
        "phpstan" => ("PHPStan", "project"),
        "psalm" => ("Psalm", "project"),
        "phpcs" => ("PHPCS", "project"),
        "phpcbf" => ("PHPCBF", "project"),
        "parallel-lint" => ("PHP lint", "project"),
        "phpmd" => ("PHPMD", "project"),
        "deptrac" => ("Deptrac", "project"),
        "behat" => ("Behat", "tests"),
        "infection" => ("Infection", "tests"),
        "pint" => ("Pint", "project"),
        "php-cs-fixer" => ("PHP-CS-Fixer", "project"),
        "rector" => ("Rector", "project"),
        _ => return None,
    };
    if family == "PHPStan" {
        if !sub.is_empty() && !sub.starts_with('-') && !["analyse", "analyze"].contains(&sub) {
            return None;
        }
        return found(
            family,
            significant(
                if ["analyse", "analyze"].contains(&sub) {
                    list.get(i + 2..).unwrap_or(&[])
                } else {
                    args
                },
                family,
            ),
            "project",
            false,
        );
    }
    if family == "PHP-CS-Fixer" {
        if !["fix", "check"].contains(&sub) {
            return None;
        }
        let fixer = sub == "fix" && !args.iter().any(|w| w.value == "--dry-run");
        return found(
            family,
            significant(list.get(i + 2..).unwrap_or(&[]), ""),
            scope,
            fixer,
        );
    }
    if family == "Rector" {
        return found(
            family,
            significant(
                if sub == "process" {
                    list.get(i + 2..).unwrap_or(&[])
                } else {
                    args
                },
                "",
            ),
            scope,
            !args.iter().any(|w| w.value == "--dry-run"),
        );
    }
    let fixer = family == "PHPCBF"
        || family == "Pint" && !args.iter().any(|w| w.value == "--test")
        || family == "Psalm" && args.iter().any(|w| w.value.starts_with("--alter"));
    found(
        family,
        significant(args, family),
        if scope == "tests" {
            "all tests"
        } else {
            "project"
        },
        fixer,
    )
}
fn assignment(v: &str) -> bool {
    is_env_assignment_prefix(v)
}
fn setup(v: &str) -> bool {
    [
        "cd",
        "pushd",
        "popd",
        "export",
        "set",
        "source",
        ".",
        "Set-Location",
    ]
    .into_iter()
    .any(|kw| starts_with_keyword(v, kw, false))
        || v.starts_with("$env:")
        || is_pure_env_assignments(v)
}
fn harmless(v: &str) -> bool {
    ["echo", "printf", "true", ":"]
        .into_iter()
        .any(|kw| starts_with_keyword(v, kw, false))
        || (is_exit_status_var(v)
            && v.strip_prefix("exit $")
                .and_then(|r| {
                    r.strip_prefix('?')
                        .or_else(|| r.strip_prefix("LASTEXITCODE"))
                })
                .is_some_and(|after| {
                    after.is_empty() || after.chars().next().is_some_and(char::is_whitespace)
                }))
}
fn strip_outer(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.into()
    }
}
fn unwrap_shell(raw: &str) -> String {
    let mut text = raw.to_owned();
    for _ in 0..16 {
        let list = words(&text);
        let Some(first) = list.first() else { break };
        let program = exe(first);
        let next = if ["sh", "bash", "zsh"].contains(&program.as_str())
            && list
                .get(1)
                .is_some_and(|w| ["-c", "-lc"].contains(&w.value.as_str()))
            && list.len() == 3
        {
            Some(list[2].value.clone())
        } else if ["pwsh", "powershell"].contains(&program.as_str()) {
            list.iter()
                .position(|w| {
                    w.value.eq_ignore_ascii_case("-Command") || w.value.eq_ignore_ascii_case("-c")
                })
                .map(|at| {
                    strip_outer(
                        &list[at + 1..]
                            .iter()
                            .map(|w| w.raw.clone())
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                })
        } else if program == "cmd"
            && list
                .get(1)
                .is_some_and(|w| w.value.eq_ignore_ascii_case("/c"))
        {
            Some(strip_outer(
                &list[2..]
                    .iter()
                    .map(|w| w.raw.clone())
                    .collect::<Vec<_>>()
                    .join(" "),
            ))
        } else {
            None
        };
        let Some(next) = next else { break };
        if next == text {
            break;
        }
        text = next.trim().into();
    }
    text
}
pub fn runnable(command: &str) -> bool {
    if command.trim().is_empty()
        || command.len() > 4000
        || command.chars().any(|c| c < ' ')
        || !command.matches('"').count().is_multiple_of(2)
        || !command.matches('\'').count().is_multiple_of(2)
    {
        return false;
    }
    let list = words(command);
    !list.is_empty()
        && list.len() <= 100
        && !list
            .iter()
            .any(|w| ["|", "||", "&&", ";", ">", ">>", "<"].contains(&w.raw.as_str()))
}
pub fn classify(raw: &str) -> V {
    let text = unwrap_shell(
        &raw.chars()
            .filter(|c| !matches!(*c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
            .collect::<String>(),
    );
    let parts = parts(&text);
    let mut removed = vec![];
    let mut selected = None;
    for (index, part) in parts.iter().enumerate() {
        if setup(&part.text) {
            continue;
        }
        let mut list = words(&part.text);
        for _ in 0..64 {
            let Some(first) = list.first().cloned() else {
                break;
            };
            let base = exe(&first);
            if assignment(&first.value) {
                list.remove(0);
                removed.push("environment".into());
            } else if first.value == "env" {
                list.remove(0);
                while list.first().is_some_and(|w| assignment(&w.value)) {
                    list.remove(0);
                }
                removed.push("environment".into());
            } else if first.value == "timeout" {
                list.remove(0);
                while list.first().is_some_and(|w| w.value.starts_with('-')) {
                    list.remove(0);
                }
                if !list.is_empty() {
                    list.remove(0);
                }
                removed.push("timeout".into());
            } else if ["time", "nice", "nohup", "command", "call"].contains(&first.value.as_str()) {
                list.remove(0);
            } else if base == "sail" {
                list.remove(0);
                removed.push("Sail".into());
                let sub = list.first().map(|w| w.value.as_str()).unwrap_or("");
                match sub {
                    "test" => {
                        list.remove(0);
                        for w in ["test", "artisan", "php"] {
                            list.insert(
                                0,
                                Word {
                                    raw: w.into(),
                                    value: w.into(),
                                },
                            );
                        }
                    }
                    "artisan" => list.insert(
                        0,
                        Word {
                            raw: "php".into(),
                            value: "php".into(),
                        },
                    ),
                    "bin" if list.len() > 1 => {
                        let v = list[1].clone();
                        list.drain(0..2);
                        list.insert(
                            0,
                            Word {
                                raw: format!("vendor/bin/{}", v.raw),
                                value: format!("vendor/bin/{}", v.value),
                            },
                        );
                    }
                    "pint" => {
                        list[0] = Word {
                            raw: "vendor/bin/pint".into(),
                            value: "vendor/bin/pint".into(),
                        };
                    }
                    _ => {}
                }
            } else if base == "ddev" || base == "lando" {
                list.remove(0);
                if list.first().is_some_and(|w| w.value == "exec") {
                    list.remove(0);
                }
                removed.push(if base == "ddev" {
                    "DDEV".into()
                } else {
                    "Lando".into()
                });
            } else if base == "docker" || base == "docker-compose" {
                let mut at =
                    if base == "docker" && list.get(1).is_some_and(|w| w.value == "compose") {
                        2
                    } else {
                        1
                    };
                if !list
                    .get(at)
                    .is_some_and(|w| ["exec", "run"].contains(&w.value.as_str()))
                {
                    break;
                }
                at += 1;
                while list.get(at).is_some_and(|w| w.value.starts_with('-')) {
                    at += if list.get(at).is_some_and(|w| {
                        ["-u", "--user", "-w", "--workdir", "-e", "--env"]
                            .contains(&w.value.as_str())
                    }) {
                        2
                    } else {
                        1
                    };
                }
                let end = (at + 1).min(list.len());
                list.drain(0..end);
                removed.push("container".into());
            } else {
                break;
            }
        }
        let mut at = list.len();
        while at > 0 {
            at -= 1;
            if is_redirect_token(&list[at].raw) {
                let skip = if redirect_takes_next(&list[at].raw) {
                    2
                } else {
                    1
                };
                let end = (at + skip).min(list.len());
                list.drain(at..end);
                removed.push("redirect".into());
            }
        }
        if let Some(found) = identify(&list) {
            selected = Some((index, found, list));
            break;
        }
    }
    let Some((index, found, list)) = selected else {
        return json!({"family":null,"target":"","key":null,"check":false,"fixer":false,"masked":null,"bare":"","removed":[],"runnable":false});
    };
    let before = &parts[..index];
    if before.iter().any(|p| {
        (["export", "set", "source", "."]
            .into_iter()
            .any(|kw| starts_with_keyword(&p.text, kw, true))
            || p.text.starts_with("$env:")
            || is_env_assignment_prefix(&p.text))
            && !p.text.contains("pipefail")
    }) {
        removed.insert(0, "environment".into());
    }
    if before.iter().any(|p| {
        ["cd", "pushd", "Set-Location"]
            .into_iter()
            .any(|kw| starts_with_word_boundary(&p.text, kw))
    }) {
        removed.insert(0, "cd".into());
    }
    let next = parts[index].next;
    let after = parts.get(index + 1).map(|p| p.text.as_str()).unwrap_or("");
    let word = words(after)
        .first()
        .map(|w| w.value.clone())
        .unwrap_or_else(|| "the next command".into());
    let shown = word
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let pipefail = before.iter().any(|p| contains_word(&p.text, "pipefail"));
    let masked = match next {
        Some("|") => {
            removed.push(format!("| {shown}"));
            if pipefail {
                None
            } else {
                Some(format!("piped to {shown}: exit status is {shown}'s"))
            }
        }
        Some("||")
            if ["true", ":", "exit 0", "echo"]
                .into_iter()
                .any(|kw| starts_with_keyword(after, kw, false)) =>
        {
            removed.push(format!("|| {word}"));
            Some(format!("|| {word}: a failure is ignored"))
        }
        Some(";") if !after.is_empty() && !is_exit_status_var(after) => {
            removed.push(format!("; {word}"));
            Some(format!(
                "followed by {word}: the last command sets the exit status"
            ))
        }
        Some("&&") if !after.is_empty() && !harmless(after) => {
            removed.push(format!("&& {word}"));
            Some(format!("followed by {word}: a failure may be its"))
        }
        Some("&") => {
            removed.push("&".into());
            Some("runs in the background: the exit status is not the check's".into())
        }
        _ => None,
    };
    let mut unique = vec![];
    for item in removed {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    let bare = list
        .iter()
        .map(|w| w.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    json!({"family":found.family,"target":found.target,"key":format!("{}|{}",found.family,found.target),"check":!found.fixer,"fixer":found.fixer,"masked":masked,"bare":bare,"removed":unique,"runnable":runnable(&bare)})
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    #[test]
    fn a_successful_pipe_does_not_certify_the_php_check() {
        let output = classify(
            "bash -lc 'cd /project && vendor/bin/phpunit --filter UserTest | tee /tmp/test.log'",
        );
        assert_eq!(output["family"], "PHPUnit");
        assert_eq!(output["check"], true);
        assert!(output["masked"]
            .as_str()
            .is_some_and(|reason| reason.contains("tee")));
        assert!(output["removed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "cd"));
    }
    #[test]
    fn pipefail_and_explicit_failure_masking_are_distinguished() {
        assert!(
            classify("set -o pipefail; vendor/bin/phpunit | tee /tmp/test.log")["masked"].is_null()
        );
        assert!(classify("vendor/bin/phpunit || true")["masked"]
            .as_str()
            .is_some());
    }
    #[test]
    fn code_modifiers_are_not_presented_as_successful_checks() {
        for command in [
            "vendor/bin/pint",
            "vendor/bin/php-cs-fixer fix",
            "vendor/bin/rector process",
        ] {
            let output = classify(command);
            assert_eq!(output["fixer"], true, "{command}");
            assert_eq!(output["check"], false, "{command}");
        }
        for command in [
            "vendor/bin/pint --test",
            "vendor/bin/php-cs-fixer fix --dry-run",
            "vendor/bin/rector process --dry-run",
        ] {
            let output = classify(command);
            assert_eq!(output["fixer"], false, "{command}");
            assert_eq!(output["check"], true, "{command}");
        }
    }
    #[test]
    fn installation_and_version_probes_are_not_verification_evidence() {
        for command in [
            "composer install",
            "composer update",
            "vendor/bin/phpunit --version",
        ] {
            let output = classify(command);
            assert!(output["family"].is_null(), "{command}");
            assert_eq!(output["check"], false, "{command}");
        }
    }
    #[test]
    fn trusted_check_launcher_rejects_shell_operators() {
        assert!(runnable("php -l \"src/User.php\""));
        assert!(!runnable("php -l src/User.php && echo done"));
        assert!(!runnable("vendor/bin/phpunit | tee output.log"));
        assert!(!runnable("php -l \"unfinished"));
    }

    #[test]
    fn pure_ascii_helpers_match_former_regex_rules() {
        assert!(is_php_executable("php"));
        assert!(is_php_executable("php8"));
        assert!(is_php_executable("php8.3"));
        assert!(!is_php_executable("php8."));
        assert!(!is_php_executable("phpunit"));
        assert!(is_redirect_token("2>&1"));
        assert!(is_redirect_token(">>/tmp/log"));
        assert!(is_redirect_token("<"));
        assert!(redirect_takes_next("2>>"));
        assert!(!redirect_takes_next("2>&1"));
        assert!(setup("FOO=1 BAR=2"));
        assert!(setup("$env:APP_ENV='test'"));
        assert!(!setup("vendor/bin/phpunit"));
        assert!(harmless("exit $?"));
        assert!(harmless("exit $LASTEXITCODE"));
        assert!(!harmless("exit 1"));
        assert_eq!(classify("php8.3 -l src/App.php")["family"], "PHP lint");
        assert_eq!(
            classify("vendor/bin/phpunit 2>&1; exit $?")["masked"],
            V::Null
        );
    }
}
