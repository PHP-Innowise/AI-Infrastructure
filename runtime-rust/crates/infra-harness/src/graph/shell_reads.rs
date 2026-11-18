//! Recover narrowly attributable reads from persisted shell summaries.
use serde_json::Value;
use std::{
    path::{Component, Path},
    sync::LazyLock,
};

static PRINT_SCRIPT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"^[1-9][0-9]*(?:,(?:[1-9][0-9]*|\$))?p$"#)
        .expect("literal sed print script pattern")
});

#[derive(Debug, Clone)]
struct Token {
    text: String,
    quoted: bool,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Quote {
    None,
    Single,
    Double,
}

struct SearchSpec {
    bool_short: &'static str,
    value_short: &'static str,
    bool_long: &'static [&'static str],
    value_long: &'static [&'static str],
    opt_eq_long: &'static [&'static str],
}

const RG_SPEC: SearchSpec = SearchSpec {
    bool_short: "nNiSsFwvclHhoquLzap0UPI.",
    value_short: "efABCmtTgjMdEr",
    bool_long: &[
        "--hidden",
        "--no-ignore",
        "--no-ignore-vcs",
        "--no-ignore-dot",
        "--no-ignore-global",
        "--no-ignore-parent",
        "--no-heading",
        "--heading",
        "--with-filename",
        "--no-filename",
        "--line-number",
        "--no-line-number",
        "--column",
        "--vimgrep",
        "--json",
        "--stats",
        "--multiline",
        "--multiline-dotall",
        "--pcre2",
        "--smart-case",
        "--ignore-case",
        "--case-sensitive",
        "--fixed-strings",
        "--word-regexp",
        "--line-regexp",
        "--invert-match",
        "--count",
        "--count-matches",
        "--files-with-matches",
        "--files-without-match",
        "--only-matching",
        "--quiet",
        "--follow",
        "--trim",
        "--no-config",
        "--one-file-system",
        "--no-messages",
        "--text",
        "--search-zip",
        "--null",
        "--passthrough",
        "--no-unicode",
        "--unicode",
        "--mmap",
        "--no-mmap",
        "--crlf",
        "--no-crlf",
        "--max-columns-preview",
        "--glob-case-insensitive",
        "--block-buffered",
        "--line-buffered",
    ],
    value_long: &[
        "--regexp",
        "--file",
        "--after-context",
        "--before-context",
        "--context",
        "--max-count",
        "--type",
        "--type-not",
        "--glob",
        "--iglob",
        "--type-add",
        "--type-clear",
        "--threads",
        "--max-columns",
        "--max-depth",
        "--max-filesize",
        "--color",
        "--colors",
        "--context-separator",
        "--field-context-separator",
        "--field-match-separator",
        "--encoding",
        "--engine",
        "--path-separator",
        "--sort",
        "--sortr",
        "--replace",
    ],
    opt_eq_long: &[],
};

const GREP_SPEC: SearchSpec = SearchSpec {
    bool_short: "EFGPivwxclLnHhoqsrRzaIUbuZTy",
    value_short: "efABCmdD",
    bool_long: &[
        "--extended-regexp",
        "--fixed-strings",
        "--basic-regexp",
        "--perl-regexp",
        "--ignore-case",
        "--no-ignore-case",
        "--invert-match",
        "--word-regexp",
        "--line-regexp",
        "--count",
        "--files-with-matches",
        "--files-without-match",
        "--line-number",
        "--with-filename",
        "--no-filename",
        "--only-matching",
        "--quiet",
        "--silent",
        "--no-messages",
        "--recursive",
        "--dereference-recursive",
        "--null",
        "--null-data",
        "--text",
        "--binary",
        "--byte-offset",
        "--initial-tab",
        "--line-buffered",
        "--no-group-separator",
    ],
    value_long: &[
        "--regexp",
        "--file",
        "--after-context",
        "--before-context",
        "--context",
        "--max-count",
        "--directories",
        "--devices",
        "--exclude",
        "--exclude-from",
        "--exclude-dir",
        "--include",
        "--label",
        "--binary-files",
        "--group-separator",
    ],
    opt_eq_long: &["--color", "--colour"],
};

const GIT_SHOW_BOOL_SHORT: &str = "puswbmqMCc";
const GIT_SHOW_VALUE_SHORT: &str = "nUSGO";
const GIT_SHOW_OPT_EQ_LONG: &[&str] = &[
    "--color",
    "--word-diff",
    "--abbrev",
    "--find-renames",
    "--find-copies",
    "--stat",
    "--submodule",
    "--notes",
    "--relative",
];
const GIT_SHOW_VALUE_LONG: &[&str] = &[
    "--max-count",
    "--unified",
    "--diff-filter",
    "--date",
    "--format",
    "--pretty",
    "--encoding",
    "--word-diff-regex",
    "--color-moved",
    "--color-moved-ws",
    "--diff-algorithm",
    "--src-prefix",
    "--dst-prefix",
];
const GIT_SHOW_BOOL_LONG: &[&str] = &[
    "--patch",
    "--no-patch",
    "--numstat",
    "--shortstat",
    "--summary",
    "--name-only",
    "--name-status",
    "--raw",
    "--abbrev-commit",
    "--no-abbrev-commit",
    "--oneline",
    "--full-index",
    "--binary",
    "--no-color",
    "--no-word-diff",
    "--ignore-all-space",
    "--ignore-space-change",
    "--ignore-space-at-eol",
    "--ignore-blank-lines",
    "--first-parent",
    "--no-notes",
    "--show-signature",
    "--quiet",
    "--no-renames",
    "--cc",
    "--combined-all-paths",
    "--full-diff",
    "--no-abbrev",
    "--no-prefix",
    "--indent-heuristic",
    "--histogram",
    "--patience",
    "--minimal",
];

fn split_and_segments(command: &str) -> Option<Vec<Vec<Token>>> {
    let mut segments: Vec<Vec<Token>> = Vec::new();
    let mut current_tokens: Vec<Token> = Vec::new();
    let mut current_word = String::new();
    let mut in_word = false;
    let mut word_quoted = false;
    let mut quote = Quote::None;
    let mut chars = command.chars().peekable();

    let flush_word = |in_word: &mut bool,
                      word_quoted: &mut bool,
                      current_word: &mut String,
                      current_tokens: &mut Vec<Token>| {
        if *in_word {
            current_tokens.push(Token {
                text: std::mem::take(current_word),
                quoted: *word_quoted,
            });
            *in_word = false;
            *word_quoted = false;
        }
    };

    while let Some(ch) = chars.next() {
        match quote {
            Quote::Single => {
                if matches!(ch, '\0' | '\n' | '\r') {
                    return None;
                }
                if ch == '\'' {
                    quote = Quote::None;
                } else {
                    current_word.push(ch);
                }
            }
            Quote::Double => {
                if matches!(ch, '\0' | '\n' | '\r') {
                    return None;
                }
                if ch == '"' {
                    quote = Quote::None;
                } else if ch == '\\' {
                    let next = chars.next()?;
                    if matches!(next, '\0' | '\n' | '\r') {
                        return None;
                    }
                    if matches!(next, '"' | '\\' | '$' | '`') {
                        current_word.push(next);
                    } else {
                        current_word.push('\\');
                        current_word.push(next);
                    }
                } else if matches!(ch, '$' | '`') {
                    return None;
                } else {
                    current_word.push(ch);
                }
            }
            Quote::None => {
                if ch.is_control() && ch != '\t' {
                    return None;
                }
                if ch == ' ' || ch == '\t' {
                    flush_word(
                        &mut in_word,
                        &mut word_quoted,
                        &mut current_word,
                        &mut current_tokens,
                    );
                } else if ch == '\'' {
                    in_word = true;
                    word_quoted = true;
                    quote = Quote::Single;
                } else if ch == '"' {
                    in_word = true;
                    word_quoted = true;
                    quote = Quote::Double;
                } else if ch == '&' {
                    if chars.next() != Some('&') || chars.peek() == Some(&'&') {
                        return None;
                    }
                    flush_word(
                        &mut in_word,
                        &mut word_quoted,
                        &mut current_word,
                        &mut current_tokens,
                    );
                    if current_tokens.is_empty() {
                        return None;
                    }
                    segments.push(std::mem::take(&mut current_tokens));
                } else if matches!(
                    ch,
                    ';' | '|'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '`'
                        | '$'
                        | '~'
                        | '{'
                        | '}'
                        | '\\'
                        | '#'
                        | '*'
                        | '?'
                        | '['
                        | ']'
                        | '…'
                ) {
                    return None;
                } else {
                    in_word = true;
                    current_word.push(ch);
                }
            }
        }
    }

    if quote != Quote::None {
        return None;
    }
    flush_word(
        &mut in_word,
        &mut word_quoted,
        &mut current_word,
        &mut current_tokens,
    );
    if current_tokens.is_empty() {
        return None;
    }
    segments.push(current_tokens);
    Some(segments)
}

fn is_safe_path_token(raw: &str) -> bool {
    let path = raw.strip_prefix("./").unwrap_or(raw);
    if path.is_empty() || raw.starts_with('-') || path.starts_with('/') {
        return false;
    }
    if path.chars().any(|ch| {
        ch.is_control()
            || matches!(
                ch,
                '*' | '?'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '~'
                    | '$'
                    | '`'
                    | '\\'
                    | '…'
                    | '|'
                    | ';'
                    | '&'
                    | '<'
                    | '>'
            )
    }) {
        return false;
    }
    let parsed = Path::new(path);
    !parsed.is_absolute()
        && !parsed.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn clean_operand_path(raw: &str) -> Option<String> {
    is_safe_path_token(raw).then(|| raw.to_owned())
}

fn parse_sed(args: &[Token]) -> Option<Vec<String>> {
    let mut quiet = false;
    let mut scripts: Vec<&str> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut after_double_dash = false;
    let mut idx = 0;

    while idx < args.len() {
        let tok = &args[idx].text;
        if !after_double_dash && tok == "--" {
            after_double_dash = true;
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with("--") {
            match tok.as_str() {
                "--quiet" | "--silent" => quiet = true,
                "--regexp-extended" | "--unbuffered" => {}
                "--expression" => {
                    idx += 1;
                    scripts.push(args.get(idx)?.text.as_str());
                }
                _ if tok.starts_with("--expression=") => {
                    scripts.push(tok.strip_prefix("--expression=")?);
                }
                _ => return None,
            }
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with('-') && tok.len() > 1 {
            let short = &tok[1..];
            for (pos, flag) in short.char_indices() {
                match flag {
                    'n' => quiet = true,
                    'E' | 'r' | 'u' => {}
                    'e' => {
                        let rest = &short[pos + flag.len_utf8()..];
                        if !rest.is_empty() {
                            scripts.push(rest);
                        } else {
                            idx += 1;
                            scripts.push(args.get(idx)?.text.as_str());
                        }
                        break;
                    }
                    _ => return None,
                }
            }
            idx += 1;
            continue;
        }
        if scripts.is_empty() && !after_double_dash {
            scripts.push(tok.as_str());
        } else {
            files.push(clean_operand_path(tok)?);
        }
        idx += 1;
    }

    if !quiet
        || scripts.is_empty()
        || !scripts.iter().all(|s| PRINT_SCRIPT.is_match(s))
        || files.is_empty()
    {
        return None;
    }
    Some(files)
}

fn parse_cat(args: &[Token]) -> Option<Vec<String>> {
    const BOOL_LONG: &[&str] = &[
        "--show-all",
        "--number-nonblank",
        "--show-ends",
        "--number",
        "--squeeze-blank",
        "--show-tabs",
        "--show-nonprinting",
    ];
    let mut files = Vec::new();
    let mut after_double_dash = false;

    for arg in args {
        let tok = &arg.text;
        if !after_double_dash && tok == "--" {
            after_double_dash = true;
            continue;
        }
        if !after_double_dash && tok.starts_with("--") {
            if !BOOL_LONG.contains(&tok.as_str()) {
                return None;
            }
            continue;
        }
        if !after_double_dash && tok.starts_with('-') && tok.len() > 1 {
            if !tok[1..].chars().all(|c| "AbeEnstTuv".contains(c)) {
                return None;
            }
            continue;
        }
        files.push(clean_operand_path(tok)?);
    }

    (!files.is_empty()).then_some(files)
}

fn is_valid_count(raw: &str) -> bool {
    let rest = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    matches!(
        &rest[digits..],
        "" | "b"
            | "k"
            | "K"
            | "kB"
            | "KB"
            | "KiB"
            | "m"
            | "M"
            | "MB"
            | "MiB"
            | "g"
            | "G"
            | "GB"
            | "GiB"
            | "t"
            | "T"
            | "TB"
            | "TiB"
    )
}

fn parse_head_tail(args: &[Token], is_tail: bool) -> Option<Vec<String>> {
    let mut files = Vec::new();
    let mut after_double_dash = false;
    let mut idx = 0;

    while idx < args.len() {
        let tok = &args[idx].text;
        if !after_double_dash && tok == "--" {
            after_double_dash = true;
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with("--") {
            match tok.as_str() {
                "--quiet" | "--silent" | "--verbose" | "--zero-terminated" => {}
                "--lines" | "--bytes" => {
                    idx += 1;
                    if !is_valid_count(args.get(idx)?.text.as_str()) {
                        return None;
                    }
                }
                _ if tok.starts_with("--lines=") || tok.starts_with("--bytes=") => {
                    let (_, count) = tok.split_once('=')?;
                    if !is_valid_count(count) {
                        return None;
                    }
                }
                _ => return None,
            }
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with('-') && tok.len() > 1 {
            let short = &tok[1..];
            if short.bytes().all(|b| b.is_ascii_digit()) {
                idx += 1;
                continue;
            }
            for (pos, flag) in short.char_indices() {
                match flag {
                    'q' | 'v' | 'z' => {}
                    'n' | 'c' => {
                        let rest = &short[pos + flag.len_utf8()..];
                        let count = if !rest.is_empty() {
                            rest
                        } else {
                            idx += 1;
                            args.get(idx)?.text.as_str()
                        };
                        if !is_valid_count(count) {
                            return None;
                        }
                        break;
                    }
                    _ => return None,
                }
            }
            idx += 1;
            continue;
        }
        if is_tail && !after_double_dash && tok.starts_with('+') {
            return None;
        }
        files.push(clean_operand_path(tok)?);
        idx += 1;
    }

    (!files.is_empty()).then_some(files)
}

fn parse_search(spec: &SearchSpec, args: &[Token]) -> Option<Vec<String>> {
    let mut has_explicit_pattern = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut after_double_dash = false;
    let mut idx = 0;

    while idx < args.len() {
        let tok = &args[idx].text;
        if !after_double_dash && tok == "--" {
            after_double_dash = true;
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with("--") {
            if let Some((flag, _)) = tok.split_once('=') {
                if matches!(flag, "--regexp" | "--file") {
                    has_explicit_pattern = true;
                } else if !spec.value_long.contains(&flag) && !spec.opt_eq_long.contains(&flag) {
                    return None;
                }
            } else if spec.value_long.contains(&tok.as_str()) {
                if matches!(tok.as_str(), "--regexp" | "--file") {
                    has_explicit_pattern = true;
                }
                idx += 1;
                if idx >= args.len() {
                    return None;
                }
            } else if !spec.bool_long.contains(&tok.as_str())
                && !spec.opt_eq_long.contains(&tok.as_str())
            {
                return None;
            }
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with('-') && tok.len() > 1 {
            let short = &tok[1..];
            for (pos, flag) in short.char_indices() {
                if spec.value_short.contains(flag) {
                    if matches!(flag, 'e' | 'f') {
                        has_explicit_pattern = true;
                    }
                    let rest = &short[pos + flag.len_utf8()..];
                    if rest.is_empty() {
                        idx += 1;
                        if idx >= args.len() {
                            return None;
                        }
                    }
                    break;
                } else if !spec.bool_short.contains(flag) {
                    return None;
                }
            }
            idx += 1;
            continue;
        }
        positional.push(tok.as_str());
        idx += 1;
    }

    let targets = if has_explicit_pattern {
        positional.as_slice()
    } else {
        positional.get(1..)?
    };
    if targets.is_empty() {
        return None;
    }
    targets
        .iter()
        .map(|path| clean_operand_path(path))
        .collect()
}

fn git_rev_path(spec: &str) -> Option<String> {
    if spec.starts_with(":/") {
        return None;
    }
    let path = if let Some(rest) = spec.strip_prefix(':') {
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && matches!(bytes[0], b'0'..=b'3') && bytes[1] == b':' {
            &rest[2..]
        } else {
            rest
        }
    } else {
        let (_, path) = spec.split_once(':')?;
        path
    };
    clean_operand_path(path)
}

fn parse_git_show(args: &[Token]) -> Option<Vec<String>> {
    let mut idx = 0;
    while idx < args.len() {
        if matches!(
            args[idx].text.as_str(),
            "--no-pager" | "--literal-pathspecs" | "--no-optional-locks"
        ) {
            idx += 1;
        } else {
            break;
        }
    }
    if args.get(idx)?.text != "show" {
        return None;
    }
    idx += 1;

    let mut files = Vec::new();
    let mut after_double_dash = false;

    while idx < args.len() {
        let tok = &args[idx].text;
        if !after_double_dash && tok == "--" {
            after_double_dash = true;
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with("--") {
            if let Some((flag, _)) = tok.split_once('=') {
                if !GIT_SHOW_OPT_EQ_LONG.contains(&flag) && !GIT_SHOW_VALUE_LONG.contains(&flag) {
                    return None;
                }
            } else if GIT_SHOW_VALUE_LONG.contains(&tok.as_str()) {
                idx += 1;
                if idx >= args.len() {
                    return None;
                }
            } else if !GIT_SHOW_OPT_EQ_LONG.contains(&tok.as_str())
                && !GIT_SHOW_BOOL_LONG.contains(&tok.as_str())
            {
                return None;
            }
            idx += 1;
            continue;
        }
        if !after_double_dash && tok.starts_with('-') && tok.len() > 1 {
            let short = &tok[1..];
            for (pos, flag) in short.char_indices() {
                if GIT_SHOW_VALUE_SHORT.contains(flag) {
                    let rest = &short[pos + flag.len_utf8()..];
                    if rest.is_empty() {
                        idx += 1;
                        if idx >= args.len() {
                            return None;
                        }
                    }
                    break;
                } else if !GIT_SHOW_BOOL_SHORT.contains(flag) {
                    return None;
                }
            }
            idx += 1;
            continue;
        }
        if after_double_dash {
            files.push(clean_operand_path(tok)?);
        } else if tok.contains(':') {
            files.push(git_rev_path(tok)?);
        }
        idx += 1;
    }

    (!files.is_empty()).then_some(files)
}

fn extract_segment(tokens: &[Token]) -> Option<Vec<String>> {
    let (cmd, args) = tokens.split_first()?;
    if cmd.quoted {
        return None;
    }
    match cmd.text.as_str() {
        "sed" => parse_sed(args),
        "cat" => parse_cat(args),
        "head" => parse_head_tail(args, false),
        "tail" => parse_head_tail(args, true),
        "rg" => parse_search(&RG_SPEC, args),
        "grep" => parse_search(&GREP_SPEC, args),
        "git" => parse_git_show(args),
        _ => None,
    }
}

fn operands(command: &str) -> Option<Vec<String>> {
    // Each command must match independently. A failed or partial chain
    // cannot tell which commands ran successfully.
    let segments = split_and_segments(command)?;
    let mut out = Vec::new();
    for segment in segments {
        out.extend(extract_segment(&segment)?);
    }
    (!out.is_empty()).then_some(out)
}

pub(crate) fn completed_command(
    id: u64,
    detail: &str,
    workspace: &Path,
) -> Option<(u64, Vec<String>)> {
    if id == 0 || workspace.as_os_str().is_empty() {
        return None;
    }
    let paths = operands(detail)?;
    let root = workspace.canonicalize().ok()?;
    let mut files: Vec<String> = Vec::new();
    for path in paths {
        let joined = workspace.join(&path);
        let Ok(resolved) = joined.canonicalize() else {
            continue;
        };
        if !resolved.is_file() || !resolved.starts_with(&root) {
            continue;
        }
        let Ok(relative) = joined.strip_prefix(workspace) else {
            continue;
        };
        let Ok(canonical_relative) = resolved.strip_prefix(&root) else {
            continue;
        };
        // Do not attribute a symlink's contents to the lexical path.
        if relative == canonical_relative
            && let Some(clean) = relative
                .components()
                .collect::<std::path::PathBuf>()
                .to_str()
                .map(|p| p.replace('\\', "/"))
            && !files.contains(&clean)
        {
            files.push(clean);
        }
    }
    (!files.is_empty()).then_some((id, files))
}

pub(crate) fn completed(event: &Value, workspace: &Path) -> Option<(u64, Vec<String>)> {
    let is_tool =
        event["kind"] == "tool" || (event["kind"] == "agent_activity" && event["type"] == "tool");
    if !is_tool
        || event["tool"] != "Shell"
        || event["state"] != "completed"
        || event["ok"] != true
        || event["exit_code"] != 0
        || event["outcome"] == "not_run"
        || workspace.as_os_str().is_empty()
    {
        return None;
    }
    let id = event["id"].as_u64().filter(|id| *id > 0)?;
    let detail = event["detail"].as_str()?;
    completed_command(id, detail, workspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_literal_prints_in_a_successful_completion_are_evidence() {
        let project = tempfile::tempdir().unwrap();
        for name in ["a.php", "b.php"] {
            std::fs::write(project.path().join(name), "<?php").unwrap();
        }
        let mut step = json!({"kind":"tool","tool":"Shell","state":"completed","ok":true,"exit_code":0,"id":9,
            "detail":"sed -n '2p' ./a.php && sed -n '1,4p' b.php"});
        assert_eq!(
            completed(&step, project.path()),
            Some((9, vec!["a.php".into(), "b.php".into()]))
        );
        for command in [
            "sed -n '1p' *.php",
            "sed -n '1p' a.php | head",
            "sed -n '1p' a.php …",
            "cd src && sed -n '1p' a.php",
            "sed -n '1p' ../a.php",
            "sed -n '1p' a.php # comment",
            "sed -n '1p' a.php && echo done",
            "sed -n '1p' a.php >/dev/null",
        ] {
            assert!(operands(command).is_none(), "{command}");
        }
        step["exit_code"] = json!(1);
        assert!(completed(&step, project.path()).is_none());
        step["exit_code"] = json!(0);
        step["state"] = json!("started");
        assert!(completed(&step, project.path()).is_none());
    }

    #[test]
    fn safe_read_commands_attribute_files_without_false_positives() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join("src")).unwrap();
        for name in ["a.php", "b.php", "src/lib.rs", "src/main.rs"] {
            std::fs::write(project.path().join(name), "<?php\nfn main() {}\n").unwrap();
        }

        // cat, head, tail
        assert_eq!(
            completed_command(1, "cat -n ./a.php src/lib.rs", project.path()),
            Some((1, vec!["a.php".into(), "src/lib.rs".into()]))
        );
        assert_eq!(
            completed_command(2, "head -n 20 src/lib.rs", project.path()),
            Some((2, vec!["src/lib.rs".into()]))
        );
        assert_eq!(
            completed_command(
                3,
                "head -25 src/main.rs && tail -n +5 a.php",
                project.path()
            ),
            Some((3, vec!["src/main.rs".into(), "a.php".into()]))
        );
        assert_eq!(
            completed_command(4, "sed -n '10,$p' -- src/lib.rs", project.path()),
            Some((4, vec!["src/lib.rs".into()]))
        );

        // rg & grep: flag arguments and positional patterns are not misattributed as files
        assert_eq!(
            completed_command(
                5,
                "rg -n -C 3 'fn main' src/main.rs src/lib.rs",
                project.path()
            ),
            Some((5, vec!["src/main.rs".into(), "src/lib.rs".into()]))
        );
        assert_eq!(
            completed_command(
                6,
                "rg -n -g '*.rs' --color always a.php src/lib.rs",
                project.path()
            ),
            Some((6, vec!["src/lib.rs".into()]))
        );
        assert_eq!(
            completed_command(7, "rg -e 'a && b' -e 'fn' a.php", project.path()),
            Some((7, vec!["a.php".into()]))
        );
        assert_eq!(
            completed_command(8, "grep -rnE 'foo|bar' src/main.rs", project.path()),
            Some((8, vec!["src/main.rs".into()]))
        );
        assert_eq!(
            completed_command(9, "rg -r replacement 'needle' src/lib.rs", project.path()),
            Some((9, vec!["src/lib.rs".into()]))
        );

        // git show: rev:path and -- path
        assert_eq!(
            completed_command(10, "git --no-pager show HEAD:./src/lib.rs", project.path()),
            Some((10, vec!["src/lib.rs".into()]))
        );
        assert_eq!(
            completed_command(
                11,
                "git show --stat abc1234 -- a.php src/main.rs",
                project.path()
            ),
            Some((11, vec!["a.php".into(), "src/main.rs".into()]))
        );
        assert_eq!(
            completed_command(12, "git show :0:b.php", project.path()),
            Some((12, vec!["b.php".into()]))
        );

        // Chained deduplication
        assert_eq!(
            completed_command(
                13,
                "head -n 10 a.php && tail -n 10 a.php && git show HEAD:a.php",
                project.path()
            ),
            Some((13, vec!["a.php".into()]))
        );

        // Directories-only searches or pattern-only searches produce no false file read
        assert!(completed_command(14, "rg 'fn main' src", project.path()).is_none());
        assert!(completed_command(15, "rg a.php", project.path()).is_none());
        assert!(completed_command(16, "grep -rn a.php", project.path()).is_none());

        // Unsafe, mutating, piped, redirected, or ambiguous invocations are rejected
        for unsafe_cmd in [
            "cat",
            "cat -",
            "cat a.php | grep foo",
            "rg 'fn' src/main.rs | head -n 5",
            "grep -rn foo src/main.rs > out.txt",
            "cat a.php; rm -rf /",
            "cat a.php &",
            "cat \"$(echo a.php)\"",
            "cat `echo a.php`",
            "head -n a.php",
            "tail -f a.php",
            "tail +10 a.php",
            "sed -i 's/a/b/' a.php",
            "sed -n 's/a/b/w evil' a.php",
            "rg --files src/lib.rs",
            "rg --pre cat 'fn' src/lib.rs",
            "rg -e foo '*.php'",
            "git show HEAD",
            "git show --stat abc1234",
            "git show :/a.php",
            "git -C src show HEAD:lib.rs",
            "git diff HEAD -- a.php",
            "git show HEAD:../a.php",
            "cat a.php …",
        ] {
            assert!(
                completed_command(99, unsafe_cmd, project.path()).is_none(),
                "expected rejection for: {unsafe_cmd}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_attributed_by_lexical_path() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("real.rs"), "pub fn ok() {}\n").unwrap();
        std::fs::write(outside.path().join("secret.rs"), "pub fn secret() {}\n").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.rs"),
            project.path().join("link_out.rs"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            project.path().join("real.rs"),
            project.path().join("link_in.rs"),
        )
        .unwrap();

        assert!(completed_command(1, "cat link_out.rs", project.path()).is_none());
        assert!(completed_command(2, "cat link_in.rs", project.path()).is_none());
        assert_eq!(
            completed_command(3, "cat real.rs", project.path()),
            Some((3, vec!["real.rs".into()]))
        );
    }
}
