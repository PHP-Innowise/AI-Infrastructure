//! Shell syntax recognition for the native hook guard; this module never evaluates text.
use crate::TailTrim;
use infra_core::{Result, RuntimeError};
use regex::Regex;
#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub words: Vec<String>,
    pub redirects: Vec<(String, String)>,
    pub document: String,
    pub depth: usize,
    pub parent: Option<usize>,
}
struct Parser {
    chars: Vec<char>,
    at: usize,
    segments: Vec<Segment>,
    complex: bool,
}
impl Parser {
    fn new(text: &str) -> Self {
        Self {
            chars: text
                .chars()
                .map(|c| {
                    if ['\u{1d}', '\u{1e}', '\u{1f}'].contains(&c) {
                        ' '
                    } else {
                        c
                    }
                })
                .collect(),
            at: 0,
            segments: vec![],
            complex: false,
        }
    }
    fn peek(&self, n: usize) -> Option<char> {
        self.chars.get(self.at + n).copied()
    }
    fn image(&self, start: usize) -> String {
        self.chars[start..self.at.min(self.chars.len())]
            .iter()
            .collect()
    }
    fn segment(&mut self, depth: usize, parent: Option<usize>) -> usize {
        let index = self.segments.len();
        self.segments.push(Segment {
            depth,
            parent,
            ..Default::default()
        });
        index
    }
    fn push(
        &mut self,
        index: usize,
        word: &mut String,
        have: &mut bool,
        redirect: &mut String,
        documents: &mut Vec<(usize, String, String)>,
    ) {
        if !*have {
            return;
        }
        let value = std::mem::take(word);
        match redirect.as_str() {
            "" => self.segments[index].words.push(value),
            "<<" | "<<-" => documents.push((index, redirect.clone(), value)),
            "<<<" => {
                self.segments[index].document.push('\u{1f}');
                self.segments[index].document.push_str(&value);
            }
            _ => self.segments[index]
                .redirects
                .push((redirect.clone(), value)),
        }
        redirect.clear();
        *have = false;
    }
    fn heredocs(&mut self, documents: &mut Vec<(usize, String, String)>) {
        for (index, op, delimiter) in std::mem::take(documents) {
            let mut body = String::new();
            while self.at < self.chars.len() {
                let start = self.at;
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.at += 1;
                }
                let mut line = self.image(start);
                if self.peek(0) == Some('\n') {
                    self.at += 1;
                }
                if line.ends_with('\r') {
                    line.pop();
                }
                if op == "<<-" {
                    line = line.trim_start_matches('\t').into();
                }
                if line == delimiter {
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            self.segments[index].document.push('\u{1f}');
            self.segments[index].document.push_str(&body);
        }
    }
    fn dollar(
        &mut self,
        word: &mut String,
        depth: usize,
        parent: usize,
        quoted: bool,
        nest: usize,
    ) {
        let start = self.at;
        match self.peek(1) {
            Some('(') => {
                self.at += 2;
                if self.peek(0) == Some('(') {
                    while self.at < self.chars.len() {
                        if self.peek(0) == Some(')') && self.peek(1) == Some(')') {
                            self.at += 2;
                            break;
                        }
                        self.at += 1;
                    }
                } else {
                    self.parse(Some(')'), depth + 1, Some(parent), nest + 1);
                }
                word.push_str(&self.image(start));
            }
            Some('{') => {
                self.at += 2;
                while self.peek(0).is_some_and(|c| c != '}') {
                    self.at += 1;
                }
                if self.peek(0).is_some() {
                    self.at += 1;
                }
                word.push_str(&self.image(start));
            }
            Some('\'') if !quoted => {
                self.at += 2;
                while let Some(c) = self.peek(0) {
                    self.at += 1;
                    if c == '\'' {
                        break;
                    }
                    if c == '\\' {
                        if let Some(next) = self.peek(0) {
                            self.at += 1;
                            word.push(match next {
                                'n' => '\n',
                                't' => '\t',
                                _ => next,
                            });
                        }
                    } else {
                        word.push(c);
                    }
                }
            }
            Some('"') if !quoted => {
                self.at += 1;
            }
            _ => {
                word.push('$');
                self.at += 1;
            }
        }
    }
    fn double(&mut self, word: &mut String, depth: usize, parent: usize, nest: usize) {
        while let Some(c) = self.peek(0) {
            match c {
                '"' => {
                    self.at += 1;
                    return;
                }
                '\\' => {
                    self.at += 1;
                    if let Some(next) = self.peek(0) {
                        self.at += 1;
                        if next == '\n' {
                            continue;
                        }
                        if !['$', '\u{60}', '"', '\\'].contains(&next) {
                            word.push('\\');
                        }
                        word.push(next);
                    }
                }
                '$' => self.dollar(word, depth, parent, true, nest),
                '\u{60}' => {
                    let start = self.at;
                    self.at += 1;
                    self.parse(Some('\u{60}'), depth + 1, Some(parent), nest + 1);
                    word.push_str(&self.image(start));
                }
                _ => {
                    word.push(c);
                    self.at += 1;
                }
            }
        }
    }
    fn parse(&mut self, stop: Option<char>, depth: usize, parent: Option<usize>, nest: usize) {
        if nest > 100 || self.segments.len() > 100000 {
            self.complex = true;
            self.at = self.chars.len();
            return;
        }
        let mut index = self.segment(depth, parent);
        let (mut word, mut redirect) = (String::new(), String::new());
        let mut have = false;
        let mut documents = vec![];
        while let Some(c) = self.peek(0) {
            if stop == Some(c) && (c == ')' || c == '\u{60}') {
                self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
                self.at += 1;
                return;
            }
            match c {
                '\n' => {
                    self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
                    self.at += 1;
                    self.heredocs(&mut documents);
                    index = self.segment(depth, parent);
                }
                ' ' | '\t' | '\r' => {
                    self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
                    self.at += 1;
                }
                ';' | '&' | '|' => {
                    self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
                    if c == '&' && self.peek(1) == Some('>') {
                        redirect = ">".into();
                        self.at += 2;
                        if self.peek(0) == Some('>') {
                            self.at += 1;
                        }
                        continue;
                    }
                    self.at += 1;
                    if matches!(
                        (c, self.peek(0)),
                        ('&', Some('&'))
                            | ('|', Some('|'))
                            | ('|', Some('&'))
                            | (';', Some(';'))
                            | (';', Some('&'))
                    ) {
                        self.at += 1;
                    }
                    index = self.segment(depth, parent);
                }
                '(' => {
                    if have {
                        word.push(c);
                        self.at += 1;
                    } else {
                        self.at += 1;
                        self.parse(Some(')'), depth, parent, nest + 1);
                        index = self.segment(depth, parent);
                    }
                }
                ')' => {
                    self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
                    self.at += 1;
                    index = self.segment(depth, parent);
                }
                '\u{60}' => {
                    let start = self.at;
                    self.at += 1;
                    self.parse(Some('\u{60}'), depth + 1, Some(index), nest + 1);
                    word.push_str(&self.image(start));
                    have = true;
                }
                '<' | '>' => {
                    if self.peek(1) == Some('(') {
                        self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
                        let start = self.at;
                        self.at += 2;
                        self.parse(Some(')'), depth + 1, Some(index), nest + 1);
                        word = self.image(start);
                        have = true;
                        continue;
                    }
                    if have && !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                        word.clear();
                        have = false;
                    } else {
                        self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
                    }
                    self.at += 1;
                    redirect = c.to_string();
                    if self.peek(0) == Some(c) {
                        self.at += 1;
                        redirect.push(c);
                        if c == '<' && self.peek(0) == Some('<') {
                            self.at += 1;
                            redirect.push('<');
                        } else if c == '<' && self.peek(0) == Some('-') {
                            self.at += 1;
                            redirect.push('-');
                        }
                    } else if self
                        .peek(0)
                        .is_some_and(|next| ['&', '|'].contains(&next) || c == '<' && next == '>')
                    {
                        self.at += 1;
                    }
                }
                '\'' => {
                    self.at += 1;
                    while let Some(next) = self.peek(0) {
                        self.at += 1;
                        if next == '\'' {
                            break;
                        }
                        word.push(next);
                    }
                    have = true;
                }
                '"' => {
                    self.at += 1;
                    self.double(&mut word, depth, index, nest);
                    have = true;
                }
                '\\' => {
                    self.at += 1;
                    if let Some(next) = self.peek(0) {
                        self.at += 1;
                        if next != '\n' {
                            word.push(next);
                            have = true;
                        }
                    }
                }
                '$' => {
                    self.dollar(&mut word, depth, index, false, nest);
                    have = true;
                }
                '#' => {
                    if have {
                        word.push(c);
                        self.at += 1;
                    } else {
                        while self.peek(0).is_some_and(|next| next != '\n') {
                            self.at += 1;
                        }
                    }
                }
                _ => {
                    word.push(c);
                    have = true;
                    self.at += 1;
                }
            }
        }
        self.push(index, &mut word, &mut have, &mut redirect, &mut documents);
    }
}
pub fn split(text: &str) -> Result<Vec<Segment>> {
    let mut parser = Parser::new(text);
    parser.parse(None, 0, None, 0);
    if parser.complex {
        return Err(RuntimeError::policy(
            "command too complex - nesting too deep to check",
        ));
    }
    Ok(parser.segments)
}
pub fn words(text: &str) -> Result<Vec<String>> {
    let segments = split(text)?;
    if segments.len() != 1 || !segments[0].redirects.is_empty() || !segments[0].document.is_empty()
    {
        return Err(RuntimeError::invalid("Expected one simple command"));
    }
    Ok(segments[0].words.clone())
}
fn base(value: &str) -> &str {
    if value.len() > 512 {
        ""
    } else {
        value.rsplit('/').next().unwrap_or(value)
    }
}
fn re(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).map_err(|_| RuntimeError::invalid("Invalid native hook policy expression"))
}
fn assign(value: &str) -> bool {
    re(r"^[A-Za-z_][A-Za-z0-9_]*[+]?=").is_ok_and(|p| p.is_match(value))
}
pub(crate) fn short(value: &str, key: char, stops: &str) -> bool {
    if !value.starts_with('-') || value.starts_with("--") || value.len() > 64 {
        return false;
    }
    for c in value.chars().skip(1) {
        if c == key {
            return true;
        }
        if stops.contains(c) {
            return false;
        }
    }
    false
}
pub(crate) fn short_value(value: &str, stops: &str) -> bool {
    if !value.starts_with('-') || value.starts_with("--") || value.len() > 64 {
        return false;
    }
    let chars = value.chars().skip(1).collect::<Vec<_>>();
    for (i, c) in chars.iter().enumerate() {
        if stops.contains(*c) {
            return i + 1 == chars.len();
        }
    }
    false
}
pub(crate) fn php_script(words: &[String], mut i: usize) -> Option<usize> {
    while i < words.len() {
        match words[i].as_str() {
            "-r" | "-R" | "-B" | "-E" | "-a" | "-i" | "-l" | "-m" | "-s" | "-v" | "-w" | "-h"
            | "--run" | "--info" => return None,
            "-f" | "-F" => return (i + 1 < words.len()).then_some(i + 1),
            "-c" | "-d" | "-S" | "-t" | "-z" | "--rf" | "--rc" | "--re" | "--rz" | "--ri" => i += 2,
            "--" => return (i + 1 < words.len()).then_some(i + 1),
            value if value.starts_with('-') => i += 1,
            _ => return Some(i),
        }
    }
    None
}
fn program(words: &[String]) -> (usize, String, usize) {
    let mut i = 0;
    while i < words.len() {
        let name = base(&words[i]);
        if assign(&words[i])
            || [
                "!", "{", "}", "if", "then", "elif", "else", "do", "while", "until", "builtin",
                "nohup",
            ]
            .contains(&name)
        {
            i += 1;
            continue;
        }
        let takes = match name {
            "command" => {
                if words
                    .get(i + 1)
                    .is_some_and(|w| ["-v", "-V"].contains(&w.as_str()))
                {
                    return (words.len(), String::new(), words.len());
                }
                ""
            }
            "sudo" => "-u,-g,-h,-p,-C,-D,-r,-t,-T,-U,-R",
            "doas" => "-u,-C",
            "env" => "-u,-C,-S",
            "nice" => "-n",
            "ionice" => "-c,-n,-p,-P,-u",
            "timeout" => "-s,-k",
            "stdbuf" => "-i,-o,-e",
            "xargs" => "-I,-n,-P,-L,-s,-d,-E,-a",
            "time" => "-f,-o",
            "exec" => "-a",
            _ => break,
        };
        i += 1;
        while i < words.len() {
            if words[i] == "--" {
                i += 1;
                break;
            }
            if !words[i].starts_with('-') {
                break;
            }
            i += if takes.split(',').any(|v| v == words[i]) {
                2
            } else {
                1
            };
        }
        if name == "timeout" {
            i += 1;
        }
    }
    let mut name = words
        .get(i)
        .map(|word| base(word).to_string())
        .unwrap_or_default();
    let mut argument = i + 1;
    if (name == "php"
        || name
            .strip_prefix("php")
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit())))
        && let Some(script) = php_script(words, argument)
    {
        name = base(&words[script]).into();
        argument = script + 1;
    }
    name = name.tail_matches(".phar").into();
    if name == "wp-cli" {
        name = "wp".into();
    }
    (i, name, argument.min(words.len()))
}
fn glob(value: &str, pattern: &str) -> bool {
    let mut out = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '[' => {
                out.push('[');
                if chars.peek() == Some(&'!') {
                    chars.next();
                    out.push('^');
                }
                for next in chars.by_ref() {
                    out.push(next);
                    if next == ']' {
                        break;
                    }
                }
            }
            _ => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    out.push('$');
    Regex::new(&out).is_ok_and(|p| p.is_match(value))
}
fn listed(value: &str, patterns: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split(',') {
        if let Some(pattern) = pattern.strip_prefix('!') {
            if glob(value, pattern) {
                return false;
            }
        } else if glob(value, pattern) {
            matched = true;
        }
    }
    matched
}
const SEARCH: &[&str] = &["grep", "egrep", "fgrep", "rg", "ag", "ack", "git"];
const LAUNCHERS: &[&str] = &[
    "ssh",
    "sshpass",
    "mosh",
    "ddev",
    "lando",
    "vagrant",
    "sail",
    "docker",
    "docker-compose",
    "podman",
    "podman-compose",
    "nerdctl",
    "kubectl",
    "oc",
    "gcloud",
    "heroku",
    "fly",
    "flyctl",
    "tmux",
    "screen",
    "watch",
    "script",
    "flock",
    "parallel",
    "nix-shell",
    "devbox",
    "distrobox",
    "toolbox",
    "multipass",
    "limactl",
];

const RULESETS: &[(&str, &str, &str, &str, bool, &str)] = &[
    (
        "core",
        "artisan,console,phinx,doctrine-migrations,doctrine",
        "--env,--environment,--em,--conn,--configuration,--db-configuration,--write-sql,--all-or-nothing,--parser,--date",
        "ecpd",
        true,
        "argv|gh|repo delete||destructive command - gh repo delete\nargv|gh|repo archive||destructive command - gh repo archive\nargv|gh|issue delete||destructive command - gh issue delete\nargv|gh|release delete||destructive command - gh release delete\nargv|composer|config|has:github-oauth*,gitlab-oauth*,gitlab-token*,bitbucket-oauth*,forgejo-token*,http-basic*,bearer*,custom-headers*|secret exposure - composer config writing an auth token\nargv|dropdb|||destructive command - dropdb\nargv|mysqladmin||has:drop|destructive command - mysqladmin drop\nread|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env,.env.local,.env.*.local|secret exposure - reading a .env file\nconsole|migrate:fresh||destructive command - console migrate:fresh\nconsole|migrate:refresh||destructive command - console migrate:refresh\nconsole|migrate:reset||destructive command - console migrate:reset\nconsole|migrate:rollback||destructive command - console migrate:rollback\nconsole|db:wipe||destructive command - console db:wipe\nconsole|orm:schema-tool:drop||destructive command - doctrine orm:schema-tool:drop\nconsole|rollback||destructive command - phinx rollback\nconsole|migrate|has:-t0,-t=0,--target=0|destructive command - phinx migrate -t 0\nconsole|migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine-migrations migrate down\nconsole|migrations:migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine-migrations migrate down\nconsole|execute|has:--down|destructive command - doctrine-migrations execute --down\nconsole|migrations:execute|has:--down|destructive command - doctrine-migrations execute --down",
    ),
    (
        "laravel",
        "artisan",
        "--env",
        "",
        false,
        "argv|gh|repo delete||destructive command - gh repo delete\nargv|gh|repo archive||destructive command - gh repo archive\nargv|gh|issue delete||destructive command - gh issue delete\nargv|gh|release delete||destructive command - gh release delete\nargv|composer|config|has:github-oauth*,gitlab-oauth*,gitlab-token*,bitbucket-oauth*,forgejo-token*,http-basic*,bearer*,custom-headers*|secret exposure - composer config writing an auth token\nargv|dropdb|||destructive command - dropdb\nargv|mysqladmin||has:drop|destructive command - mysqladmin drop\nread|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env,.env.local,.env.*.local|secret exposure - reading a .env file\nconsole|migrate:fresh||destructive command - artisan migrate:fresh\nconsole|migrate:refresh||destructive command - artisan migrate:refresh\nconsole|migrate:reset||destructive command - artisan migrate:reset\nconsole|migrate:rollback||destructive command - artisan migrate:rollback\nconsole|db:wipe||destructive command - artisan db:wipe\nconsole|db:seed|has:--force,--force=*|destructive command - artisan db:seed --force\nconsole|model:prune|lacks:--pretend|destructive command - artisan model:prune\nconsole|queue:flush||destructive command - artisan queue:flush\nconsole|queue:clear||destructive command - artisan queue:clear\nconsole|horizon:clear||destructive command - artisan horizon:clear\nconsole|env:decrypt||secret exposure - artisan env:decrypt\nconsole|config:show|arg:database,database.*,app,app.key,app.previous_keys,services,services.*,mail,mail.*,filesystems,filesystems.*,queue,queue.*,cache,cache.*,broadcasting,broadcasting.*,reverb,reverb.*,logging,logging.*,cashier,cashier.*,scout,scout.*,passport,passport.*|secret exposure - artisan config:show of credentials\nread|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env.*,!.env.example,!.env.dist,!.env.encrypted,!.env.testing,!.env.sample,!.env.*.example|secret exposure - reading a Laravel .env file",
    ),
    (
        "symfony",
        "console",
        "--env,--em,--conn,--configuration,--db-configuration,--write-sql,--all-or-nothing",
        "e",
        false,
        "argv|gh|repo delete||destructive command - gh repo delete\nargv|gh|repo archive||destructive command - gh repo archive\nargv|gh|issue delete||destructive command - gh issue delete\nargv|gh|release delete||destructive command - gh release delete\nargv|composer|config|has:github-oauth*,gitlab-oauth*,gitlab-token*,bitbucket-oauth*,forgejo-token*,http-basic*,bearer*,custom-headers*|secret exposure - composer config writing an auth token\nargv|dropdb|||destructive command - dropdb\nargv|mysqladmin||has:drop|destructive command - mysqladmin drop\nread|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env,.env.local,.env.*.local|secret exposure - reading a .env file\nconsole|doctrine:database:drop||destructive command - doctrine:database:drop\nconsole|doctrine:schema:drop||destructive command - doctrine:schema:drop\nconsole|doctrine:migrations:execute|has:--down|destructive command - doctrine:migrations:execute --down\nconsole|doctrine:migrations:migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine:migrations:migrate down\nconsole|doctrine:fixtures:load|lacks:--append|destructive command - doctrine:fixtures:load without --append\nconsole|messenger:failed:remove|has:--all|destructive command - messenger:failed:remove --all\nconsole|secrets:reveal||secret exposure - secrets:reveal\nconsole|secrets:list|has:--reveal,-r|secret exposure - secrets:list --reveal\nconsole|secrets:decrypt-to-local||secret exposure - secrets:decrypt-to-local\nconsole|debug:dotenv||secret exposure - debug:dotenv\nconsole|debug:container|has:--env-vars,--env-var,--env-var=*|secret exposure - debug:container env var values",
    ),
    (
        "wordpress",
        "artisan,console,phinx,doctrine-migrations",
        "--env,--environment,--em,--conn,--configuration,--db-configuration,--write-sql,--all-or-nothing,--parser,--date",
        "ecpd",
        true,
        "argv|gh|repo delete||destructive command - gh repo delete\nargv|gh|repo archive||destructive command - gh repo archive\nargv|gh|issue delete||destructive command - gh issue delete\nargv|gh|release delete||destructive command - gh release delete\nargv|composer|config|has:github-oauth*,gitlab-oauth*,gitlab-token*,bitbucket-oauth*,forgejo-token*,http-basic*,bearer*,custom-headers*|secret exposure - composer config writing an auth token\nargv|dropdb|||destructive command - dropdb\nargv|mysqladmin||has:drop|destructive command - mysqladmin drop\nread|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env,.env.local,.env.*.local|secret exposure - reading a .env file\nconsole|migrate:fresh||destructive command - console migrate:fresh\nconsole|migrate:refresh||destructive command - console migrate:refresh\nconsole|migrate:reset||destructive command - console migrate:reset\nconsole|migrate:rollback||destructive command - console migrate:rollback\nconsole|db:wipe||destructive command - console db:wipe\nargv|wp|db reset||destructive command - wp db reset\nargv|wp|db drop||destructive command - wp db drop\nargv|wp|db clean||destructive command - wp db clean\nargv|wp|site delete||destructive command - wp site delete\nargv|wp|site empty||destructive command - wp site empty\nargv|wp|search-replace|has:--all-tables,--all-tables-with-prefix|destructive command - wp search-replace across all tables\nargv|wp|config set||destructive command - wp config set\nargv|wp|config delete||destructive command - wp config delete\nargv|wp|config shuffle-salts||destructive command - wp config shuffle-salts\nargv|wp|config get|arg:DB_PASSWORD,AUTH_KEY,SECURE_AUTH_KEY,LOGGED_IN_KEY,NONCE_KEY,AUTH_SALT,SECURE_AUTH_SALT,LOGGED_IN_SALT,NONCE_SALT|secret exposure - wp config get of a credential\nargv|wp|config list|argsub:DB_PASSWORD,AUTH_KEY,SECURE_AUTH_KEY,LOGGED_IN_KEY,NONCE_KEY,AUTH_SALT,SECURE_AUTH_SALT,LOGGED_IN_SALT,NONCE_SALT|secret exposure - wp config list values\nread|cat,tac,nl,less,more,head,tail,bat,batcat,sed,awk,cut,grep,egrep,fgrep,rg,ag,ack|wp-config.php,wp-config-*.php,!wp-config-sample.php|secret exposure - reading wp-config.php",
    ),
    (
        "creator",
        "artisan,console,phinx,doctrine-migrations,doctrine",
        "--env,--environment,--em,--conn,--configuration,--db-configuration,--write-sql,--all-or-nothing,--parser,--date",
        "ecpd",
        true,
        "argv|gh|repo delete||destructive command - gh repo delete\nargv|gh|repo archive||destructive command - gh repo archive\nargv|gh|issue delete||destructive command - gh issue delete\nargv|gh|release delete||destructive command - gh release delete\nargv|composer|config|has:github-oauth*,gitlab-oauth*,gitlab-token*,bitbucket-oauth*,forgejo-token*,http-basic*,bearer*,custom-headers*|secret exposure - composer config writing an auth token\nargv|dropdb|||destructive command - dropdb\nargv|mysqladmin||has:drop|destructive command - mysqladmin drop\nread|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env,.env.local,.env.*.local|secret exposure - reading a .env file\nconsole|migrate:fresh||destructive command - console migrate:fresh\nconsole|migrate:refresh||destructive command - console migrate:refresh\nconsole|migrate:reset||destructive command - console migrate:reset\nconsole|migrate:rollback||destructive command - console migrate:rollback\nconsole|db:wipe||destructive command - console db:wipe\nconsole|db:seed|has:--force,--force=*|destructive command - artisan db:seed --force\nconsole|model:prune|lacks:--pretend|destructive command - artisan model:prune\nconsole|queue:flush||destructive command - artisan queue:flush\nconsole|queue:clear||destructive command - artisan queue:clear\nconsole|horizon:clear||destructive command - artisan horizon:clear\nconsole|env:decrypt||secret exposure - artisan env:decrypt\nconsole|config:show|arg:database,database.*,app,app.key,app.previous_keys,services,services.*,mail,mail.*,filesystems,filesystems.*,queue,queue.*,cache,cache.*,broadcasting,broadcasting.*,reverb,reverb.*,logging,logging.*,cashier,cashier.*,scout,scout.*,passport,passport.*|secret exposure - artisan config:show of credentials\nconsole|doctrine:database:drop||destructive command - doctrine:database:drop\nconsole|doctrine:schema:drop||destructive command - doctrine:schema:drop\nconsole|doctrine:migrations:execute|has:--down|destructive command - doctrine:migrations:execute --down\nconsole|doctrine:migrations:migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine:migrations:migrate down\nconsole|doctrine:fixtures:load|lacks:--append|destructive command - doctrine:fixtures:load without --append\nconsole|messenger:failed:remove|has:--all|destructive command - messenger:failed:remove --all\nconsole|secrets:reveal||secret exposure - secrets:reveal\nconsole|secrets:list|has:--reveal,-r|secret exposure - secrets:list --reveal\nconsole|secrets:decrypt-to-local||secret exposure - secrets:decrypt-to-local\nconsole|debug:dotenv||secret exposure - debug:dotenv\nconsole|debug:container|has:--env-vars,--env-var,--env-var=*|secret exposure - debug:container env var values\nconsole|orm:schema-tool:drop||destructive command - doctrine orm:schema-tool:drop\nconsole|rollback||destructive command - phinx rollback\nconsole|migrate|has:-t0,-t=0,--target=0|destructive command - phinx migrate -t 0\nconsole|migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine-migrations migrate down\nconsole|migrations:migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine-migrations migrate down\nconsole|execute|has:--down|destructive command - doctrine-migrations execute --down\nconsole|migrations:execute|has:--down|destructive command - doctrine-migrations execute --down\nargv|wp|db reset||destructive command - wp db reset\nargv|wp|db drop||destructive command - wp db drop\nargv|wp|db clean||destructive command - wp db clean\nargv|wp|site delete||destructive command - wp site delete\nargv|wp|site empty||destructive command - wp site empty\nargv|wp|search-replace|has:--all-tables,--all-tables-with-prefix|destructive command - wp search-replace across all tables\nargv|wp|config set||destructive command - wp config set\nargv|wp|config delete||destructive command - wp config delete\nargv|wp|config shuffle-salts||destructive command - wp config shuffle-salts\nargv|wp|config get|arg:DB_PASSWORD,AUTH_KEY,SECURE_AUTH_KEY,LOGGED_IN_KEY,NONCE_KEY,AUTH_SALT,SECURE_AUTH_SALT,LOGGED_IN_SALT,NONCE_SALT|secret exposure - wp config get of a credential\nargv|wp|config list|argsub:DB_PASSWORD,AUTH_KEY,SECURE_AUTH_KEY,LOGGED_IN_KEY,NONCE_KEY,AUTH_SALT,SECURE_AUTH_SALT,LOGGED_IN_SALT,NONCE_SALT|secret exposure - wp config list values\nread|cat,tac,nl,less,more,head,tail,bat,batcat,sed,awk,cut,grep,egrep,fgrep,rg,ag,ack|wp-config.php,wp-config-*.php,!wp-config-sample.php|secret exposure - reading wp-config.php\nread|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env.*,!.env.example,!.env.dist,!.env.encrypted,!.env.testing,!.env.sample,!.env.*.example|secret exposure - reading a Laravel .env file\nwrite|.env,.env.local,.env.*.local|destructive command - writing a .env file",
    ),
];

#[derive(Clone, Debug)]
pub struct Policy {
    pub rules: Vec<String>,
    pub entries: String,
    pub php_scripts: bool,
    pub values: String,
    pub short_values: String,
    pub generated: Option<crate::hook_policy::HookPolicy>,
}
impl Policy {
    pub fn for_framework(framework: &str) -> Self {
        let name = framework.to_ascii_lowercase();
        let name = if name.contains("creator") {
            "creator"
        } else if name.contains("laravel") {
            "laravel"
        } else if name.contains("symfony") {
            "symfony"
        } else if name.contains("wordpress") {
            "wordpress"
        } else {
            "core"
        };
        let selected = RULESETS
            .iter()
            .find(|row| row.0 == name)
            .unwrap_or(&RULESETS[0]);
        Self {
            rules: selected.5.lines().map(String::from).collect(),
            entries: selected.1.into(),
            values: selected.2.into(),
            short_values: selected.3.into(),
            php_scripts: selected.4,
            generated: None,
        }
    }
    pub fn from_hook_policy(policy: crate::hook_policy::HookPolicy) -> Self {
        let mut base = Self::for_framework("core");
        base.rules.retain(|rule| !rule.starts_with("console|"));
        base.entries.clear();
        base.values.clear();
        base.short_values.clear();
        base.php_scripts = false;
        base.generated = Some(policy);
        base
    }
    fn next_option(&self, words: &[String], i: usize) -> usize {
        let word = &words[i];
        if !word.contains('=')
            && words.get(i + 1).is_some_and(|word| !word.starts_with('-'))
            && (listed(word, &self.values) || short_value(word, &self.short_values))
        {
            i + 2
        } else {
            i + 1
        }
    }
    fn console(&self, words: &[String], start: usize) -> Option<(String, usize, usize)> {
        for j in start..words.len() {
            if words[j].chars().any(char::is_whitespace) {
                continue;
            }
            let name = base(&words[j]).tail_matches(".phar");
            let (mut at, mut any) = (None, false);
            if listed(name, &self.entries) {
                at = Some(j + 1);
            } else if name == "sail"
                && words
                    .get(j + 1)
                    .is_some_and(|word| ["art", "a"].contains(&word.as_str()))
                && listed("artisan", &self.entries)
            {
                at = Some(j + 2);
            } else if (name == "php"
                || name
                    .strip_prefix("php")
                    .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit())))
                && let Some(script) = php_script(words, j + 1)
            {
                let program = base(&words[script])
                    .tail_matches(".phar")
                    .tail_matches(".php");
                if listed(program, &self.entries) {
                    at = Some(script + 1);
                } else if self.php_scripts {
                    at = Some(script + 1);
                    any = true;
                }
            }
            if let Some(mut at) = at {
                let argument = at;
                while at < words.len() && words[at].starts_with('-') {
                    at = self.next_option(words, at);
                }
                let name = words.get(at)?.to_ascii_lowercase();
                if name.len() > 128 || any && !name.contains(':') {
                    return None;
                }
                return Some((name, at, argument));
            }
        }
        None
    }
    fn condition(
        &self,
        condition: &str,
        words: &[String],
        first: usize,
        positional: usize,
        console: Option<usize>,
    ) -> bool {
        if condition.is_empty() {
            return true;
        }
        let Some((kind, patterns)) = condition.split_once(':') else {
            return false;
        };
        match kind {
            "has" | "lacks" => {
                let found = words
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take_while(|(_, word)| word.as_str() != "--")
                    .filter(|(i, _)| Some(*i) != console)
                    .any(|(_, word)| {
                        listed(word, patterns)
                            || console.is_some()
                                && patterns.split(',').any(|pattern| {
                                    pattern.len() == 2
                                        && pattern.starts_with('-')
                                        && short(
                                            word,
                                            pattern.chars().nth(1).unwrap_or(' '),
                                            &self.short_values,
                                        )
                                })
                    });
                if kind == "has" { found } else { !found }
            }
            "arg" => {
                let mut i = positional;
                while i < words.len() && words[i].starts_with('-') {
                    i = if console.is_some() {
                        self.next_option(words, i)
                    } else {
                        i + 1
                    };
                }
                words.get(i).is_some_and(|word| listed(word, patterns))
            }
            "argsub" => {
                let operands = words
                    .iter()
                    .skip(positional)
                    .filter(|word| !word.starts_with('-'))
                    .collect::<Vec<_>>();
                operands.is_empty()
                    || operands
                        .iter()
                        .any(|word| patterns.split(',').any(|name| name.contains(word.as_str())))
            }
            _ => false,
        }
    }
    fn apply(
        &self,
        segment: &Segment,
        program: &str,
        argument: usize,
        start: usize,
    ) -> Option<String> {
        if let Some(category) = self
            .generated
            .as_ref()
            .and_then(|policy| policy.classify(&segment.words, program, argument))
        {
            return Some(category);
        }
        let console = if SEARCH.contains(&program) || ["echo", "printf"].contains(&program) {
            None
        } else {
            self.console(&segment.words, start)
        };
        for rule in &self.rules {
            let fields = rule.split('|').collect::<Vec<_>>();
            match fields.as_slice() {
                ["argv", programs, lead, condition, category]
                    if programs.split(',').any(|name| name == program) =>
                {
                    let mut i = argument;
                    let mut matched = true;
                    for word in lead.split_whitespace() {
                        while i < segment.words.len()
                            && (segment.words[i].starts_with('-')
                                || segment.words[i].starts_with('@'))
                        {
                            i += 1;
                        }
                        if segment.words.get(i).map(String::as_str) != Some(word) {
                            matched = false;
                            break;
                        }
                        i += 1;
                    }
                    if matched && self.condition(condition, &segment.words, argument, i, None) {
                        return Some((*category).into());
                    }
                }
                ["console", full, condition, category] => {
                    if let Some((typed, at, first)) = &console {
                        let typed = typed.split(':').collect::<Vec<_>>();
                        let full = full.split(':').collect::<Vec<_>>();
                        if typed.len() == full.len()
                            && typed
                                .iter()
                                .zip(full.iter())
                                .all(|(typed, full)| full.starts_with(typed))
                            && self.condition(condition, &segment.words, *first, at + 1, Some(*at))
                        {
                            return Some((*category).into());
                        }
                    }
                }
                ["read", readers, files, category] => {
                    if reads(segment, program, argument, readers, files) {
                        return Some((*category).into());
                    }
                }
                ["write", files, category]
                    if (segment.redirects.iter().any(|(op, target)| {
                        [">", ">>"].contains(&op.as_str()) && listed(base(target), files)
                    }) || program == "tee"
                        && segment
                            .words
                            .iter()
                            .skip(argument)
                            .filter(|word| !word.starts_with('-'))
                            .any(|word| listed(base(word), files))) =>
                {
                    return Some((*category).into());
                }
                _ => {}
            }
        }
        None
    }
}
fn reads(segment: &Segment, program: &str, argument: usize, readers: &str, files: &str) -> bool {
    if program == "sed" && readers.split(',').any(|reader| reader == "sed-print") {
        if segment.words.iter().skip(argument).any(|word| {
            word.starts_with("-i") || word.starts_with("--in-place") || short(word, 'i', "efl")
        }) {
            return false;
        }
    } else if !listed(program, readers) {
        return false;
    }
    let mut pattern = SEARCH.contains(&program);
    if pattern {
        for word in segment
            .words
            .iter()
            .skip(argument)
            .take_while(|word| word.as_str() != "--")
        {
            if [
                "-q",
                "-l",
                "-L",
                "-c",
                "--quiet",
                "--silent",
                "--count",
                "--files-with-matches",
                "--files-without-match",
            ]
            .contains(&word.as_str())
            {
                return false;
            }
            if word.starts_with("-e")
                || word.starts_with("-f")
                || word.starts_with("--regexp")
                || word.starts_with("--file")
            {
                pattern = false;
            }
        }
    }
    for word in segment.words.iter().skip(argument) {
        if word.starts_with('-') {
            continue;
        }
        if pattern {
            pattern = false;
            continue;
        }
        if listed(base(word), files) {
            return true;
        }
    }
    segment
        .redirects
        .iter()
        .any(|(op, target)| op == "<" && listed(base(target), files))
}
fn known(program: &str, policy: &Policy) -> bool {
    policy
        .generated
        .as_ref()
        .is_some_and(|policy| policy.knows_program(program))
        || ["git", "rm", "gh", "eval", "echo", "printf", "sed", "tee"].contains(&program)
        || SEARCH.contains(&program)
        || policy.rules.iter().any(|rule| {
            rule.split('|')
                .nth(1)
                .is_some_and(|names| names.split(',').any(|name| name == program))
        })
}
fn broad(value: &str) -> bool {
    if value.len() > 512 {
        return false;
    }
    let value = value.tail_matches('/');
    let value = value.strip_suffix("/*").unwrap_or(value).tail_matches('/');
    [
        "*",
        ".*",
        "$HOME",
        concat!("$", "{HOME}"),
        "$PWD",
        concat!("$", "{PWD}"),
        "$(pwd)",
        "\u{60}pwd\u{60}",
    ]
    .contains(&value)
        || re(r"^(?:/[^/]*|~[A-Za-z0-9._-]*|(?:\.\.?/)*\.\.?)$")
            .is_ok_and(|pattern| pattern.is_match(if value.is_empty() { "/" } else { value }))
}
fn git(words: &[String], mut i: usize, nested: &mut Vec<String>) -> Option<String> {
    let mut sub = "";
    while i < words.len() {
        let word = &words[i];
        match word.as_str() {
            "-c" | "--config-env" => {
                if words
                    .get(i + 1)
                    .is_some_and(|value| value.to_ascii_lowercase().starts_with("core.hookspath="))
                {
                    return Some("verification bypass - git -c core.hooksPath".into());
                }
                i += 2;
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" | "--super-prefix"
            | "--attr-source" => i += 2,
            value if value.starts_with("--config-env=") => {
                if value["--config-env=".len()..]
                    .to_ascii_lowercase()
                    .starts_with("core.hookspath=")
                {
                    return Some("verification bypass - git -c core.hooksPath".into());
                }
                i += 1;
            }
            value if value.starts_with('-') => i += 1,
            _ => {
                sub = word;
                i += 1;
                break;
            }
        }
    }
    let arguments = &words[i.min(words.len())..];
    let before = arguments
        .iter()
        .take_while(|word| word.as_str() != "--")
        .collect::<Vec<_>>();
    match sub {
        "push" => {
            let mut i = 0;
            let mut options = true;
            while i < arguments.len() {
                let word = &arguments[i];
                i += 1;
                if options {
                    if word == "--" {
                        options = false;
                        continue;
                    }
                    if ["--force", "--mirror", "--force-with-lease"].contains(&word.as_str())
                        || word.starts_with("--force-with-lease=")
                        || short(word, 'f', "o")
                    {
                        return Some("destructive command - git force push".into());
                    }
                    if ["--repo", "--receive-pack", "--exec", "--push-option", "-o"]
                        .contains(&word.as_str())
                    {
                        i += 1;
                        continue;
                    }
                    if word.starts_with('-') {
                        continue;
                    }
                }
                if word.starts_with('+') && word.len() > 1 {
                    return Some("destructive command - git force push (+refspec)".into());
                }
            }
        }
        "commit" => {
            let mut i = 0;
            while i < arguments.len() {
                let word = &arguments[i];
                i += 1;
                if word == "--" {
                    break;
                }
                if word == "--no-verify" {
                    return Some("verification bypass - git commit --no-verify".into());
                }
                if short(word, 'n', "mFCctSu") {
                    return Some("verification bypass - git commit -n".into());
                }
                if [
                    "--message",
                    "--file",
                    "--author",
                    "--date",
                    "--template",
                    "--reuse-message",
                    "--reedit-message",
                    "--fixup",
                    "--squash",
                    "--cleanup",
                    "--trailer",
                    "--pathspec-from-file",
                ]
                .contains(&word.as_str())
                    || short_value(word, "mFCct")
                {
                    i += 1;
                }
            }
            return None;
        }
        "reset" if before.iter().any(|word| word.as_str() == "--hard") => {
            return Some("destructive command - git reset --hard".into());
        }
        "clean"
            if before
                .iter()
                .any(|word| word.as_str() == "--force" || short(word, 'f', "e")) =>
        {
            return Some("destructive command - git clean --force".into());
        }
        "branch" => {
            let delete = before.iter().any(|word| {
                word.as_str() == "--delete" || short(word, 'd', "") || short(word, 'D', "")
            });
            let force = before.iter().any(|word| {
                word.as_str() == "--force" || short(word, 'f', "") || short(word, 'D', "")
            });
            if delete && force {
                return Some("destructive command - git branch force delete".into());
            }
        }
        "config" => {
            let (mut read, mut unset, mut key, mut value) = (false, false, false, false);
            let mut i = 0;
            while i < arguments.len() {
                let word = &arguments[i];
                i += 1;
                if [
                    "--get",
                    "--get-all",
                    "--get-regexp",
                    "--get-urlmatch",
                    "--list",
                    "-l",
                    "get",
                    "list",
                ]
                .contains(&word.as_str())
                {
                    read = true;
                } else if ["--unset", "--unset-all", "unset"].contains(&word.as_str()) {
                    unset = true;
                } else if [
                    "--file",
                    "-f",
                    "--blob",
                    "--type",
                    "--default",
                    "--comment",
                    "--value",
                ]
                .contains(&word.as_str())
                {
                    i += 1;
                } else if word.starts_with('-') {
                } else if word.eq_ignore_ascii_case("core.hookspath") {
                    key = true;
                } else if key {
                    value = true;
                }
            }
            if key && !read && (unset || value) {
                return Some("verification bypass - git config core.hooksPath".into());
            }
        }
        "submodule" => {
            let mut i = 0;
            while i < arguments.len() && arguments[i].starts_with('-') {
                i += 1;
            }
            if arguments.get(i).map(String::as_str) == Some("foreach") {
                i += 1;
                while i < arguments.len()
                    && ["--recursive", "-q", "--quiet"].contains(&arguments[i].as_str())
                {
                    i += 1;
                }
                nested.push(arguments[i..].join(" "));
            }
        }
        "log" | "grep" | "show" | "diff" | "shortlog" | "whatchanged" | "blame" | "rev-list"
        | "reflog" => return None,
        _ => {}
    }
    if before.iter().any(|word| word.as_str() == "--no-verify") {
        return Some("verification bypass - git --no-verify".into());
    }
    None
}
fn generic(
    words: &[String],
    name: &str,
    argument: usize,
    nested: &mut Vec<String>,
) -> Option<String> {
    let arguments = &words[argument.min(words.len())..];
    match name {
        "git" => git(words, argument, nested),
        "rm" => {
            let (mut recursive, mut root, mut options) = (false, false, true);
            for word in arguments {
                if options {
                    if word == "--" {
                        options = false;
                        continue;
                    }
                    if word == "--no-preserve-root" {
                        return Some("destructive command - rm --no-preserve-root".into());
                    }
                    if word == "--recursive" || short(word, 'r', "") || short(word, 'R', "") {
                        recursive = true;
                        continue;
                    }
                    if word.starts_with('-') {
                        continue;
                    }
                }
                root |= broad(word);
            }
            if recursive && root {
                Some(
                    "destructive command - recursive rm of a root, home or working-tree path"
                        .into(),
                )
            } else {
                None
            }
        }
        "gh" => {
            let (mut method, mut first) = (String::new(), String::new());
            let mut i = 0;
            while i < arguments.len() {
                let word = &arguments[i];
                if ["-X", "--method"].contains(&word.as_str()) {
                    method = arguments.get(i + 1).cloned().unwrap_or_default();
                    i += 2;
                    continue;
                }
                if let Some(value) = word
                    .strip_prefix("--method=")
                    .or_else(|| word.strip_prefix("-X"))
                {
                    method = value.into();
                } else if !word.starts_with('-') && first.is_empty() {
                    first = word.clone();
                }
                i += 1;
            }
            if first == "api" && method.eq_ignore_ascii_case("DELETE") {
                Some("destructive command - gh api DELETE".into())
            } else {
                None
            }
        }
        "eval" => {
            nested.push(arguments.join(" "));
            None
        }
        _ => None,
    }
}
fn shell_commands(segment: &Segment, start: usize, name: &str) -> Vec<String> {
    let mut output = vec![];
    let launched = LAUNCHERS.contains(&name);
    let words = &segment.words;
    for j in start..words.len() {
        match base(&words[j]) {
            "su" | "runuser" => {
                for (i, word) in words.iter().enumerate().skip(j + 1) {
                    if ["-c", "--command"].contains(&word.as_str()) {
                        if let Some(next) = words.get(i + 1) {
                            output.push(next.clone());
                        }
                        break;
                    }
                    if let Some(value) = word.strip_prefix("--command=") {
                        output.push(value.into());
                        break;
                    }
                }
            }
            "sh" | "bash" | "zsh" | "dash" | "ksh" | "ash" | "mksh" | "yash" | "fish" => {
                let mut i = j + 1;
                let (mut cflag, mut sflag) = (false, false);
                while i < words.len() {
                    let word = &words[i];
                    if ["--rcfile", "--init-file"].contains(&word.as_str()) {
                        i += 2;
                        continue;
                    }
                    if ["-", "--"].contains(&word.as_str()) {
                        i += 1;
                        break;
                    }
                    if word == "--command" {
                        cflag = true;
                    } else if word.starts_with("--") {
                        // Long options do not consume the values of short -o/-O.
                    } else if word.starts_with('-') || word.starts_with('+') {
                        if word.starts_with('-') {
                            cflag |= short(word, 'c', "");
                            sflag |= short(word, 's', "");
                        }
                        if word.len() <= 64 {
                            i += word.chars().filter(|c| *c == 'o' || *c == 'O').count();
                        }
                    } else {
                        break;
                    }
                    i += 1;
                }
                if cflag {
                    if let Some(command) = words.get(i) {
                        output.push(command.clone());
                    }
                } else if (sflag || i >= words.len()) && (j == start || launched) {
                    output.push(segment.document.clone());
                }
            }
            "ssh" if j == start || launched => {
                let mut i = j + 1;
                while i < words.len() {
                    if words[i] == "--" {
                        i += 1;
                        break;
                    }
                    if !words[i].starts_with('-') {
                        break;
                    }
                    i += if short_value(&words[i], "BbcDEeFIiJLlmOoPpQRSWw") {
                        2
                    } else {
                        1
                    };
                }
                if i + 1 >= words.len() {
                    output.push(segment.document.clone());
                }
            }
            _ => {}
        }
    }
    if launched {
        for (i, word) in words.iter().enumerate().skip(start + 1) {
            if let Some((_, value)) = word.split_once('=') {
                if word.starts_with("--") && value.chars().any(char::is_whitespace) {
                    output.push(value.into());
                }
                continue;
            }
            if !word.starts_with('-')
                && word.chars().any(char::is_whitespace)
                && !(i > 0 && words[i - 1].starts_with('-') && assign(word))
            {
                output.push(word.clone());
            }
        }
    }
    output
}
fn sql_runner(segment: &Segment) -> bool {
    segment.words.iter().any(|word| {
        [
            "mysql", "mariadb", "mysqlsh", "mycli", "psql", "pgcli", "sqlite3", "sqlcmd", "usql",
            "tinker",
        ]
        .contains(&base(word))
            || word.ends_with(":run-sql")
            || word.ends_with(":query:sql")
    }) || segment.words.windows(2).any(|pair| {
        pair[0] == "db" && ["query", "cli"].contains(&pair[1].as_str())
            || base(&pair[0]) == "artisan" && pair[1] == "db"
    })
}
fn sql(text: &str, runs: bool) -> Result<Option<String>> {
    if re(r"(?i)(?:^|[^[:alnum:]_])drop[\s\x1f]+(?:table|database|schema)(?:[^[:alnum:]_]|$)")?
        .is_match(text)
    {
        return Ok(Some("destructive SQL - DROP TABLE/DATABASE/SCHEMA".into()));
    }
    if re(r"(?i)(?:^|[^[:alnum:]_])truncate[\s\x1f]+table(?:[^[:alnum:]_]|$)")?.is_match(text)||runs&&re(r#"(?i)(?:^|[^[:alnum:]_])truncate[\s\x1f]+(?:only[\s\x1f]+)?["\x60[:alpha:]_][\]"\x60[:alnum:]_.$]*(?:\s*(?:;|,|\x1f|$)|[\s\x1f]+(?:cascade|restrict|restart|continue)(?:[^[:alnum:]_]|$))"#)?.is_match(text){return Ok(Some("destructive SQL - TRUNCATE".into()));}
    let pattern = re(r"(?i)(?:^|[^[:alnum:]_])delete[\s\x1f]+from[\s\x1f]+[^;\s\x1f][^;]*")?;
    for found in pattern.find_iter(text).take(16) {
        let statement = found.as_str();
        if re(
            r"(?i)where[\s\x1f]+(?:1\s*=\s*1|true)(?:[^[:alnum:]_]|$)|where[\s\x1f]+1\s*(?:\x1f|$)",
        )?
        .is_match(statement)
            || runs && !re(r"(?i)(?:^|[^[:alnum:]_])where(?:[^[:alnum:]_]|$)")?.is_match(statement)
        {
            return Ok(Some("destructive SQL - DELETE without a row filter".into()));
        }
    }
    Ok(None)
}
pub fn validate(command: &str, policy: &Policy) -> Result<Option<String>> {
    let mut segments = split(command)?;
    let mut memory = false;
    let memory_only = segments
        .iter()
        .filter(|segment| !segment.words.is_empty())
        .all(|segment| {
            let (_, name, argument) = program(&segment.words);
            let cli = matches!(name.as_str(), "infra" | "infra-engine")
                && segment.words.get(argument).map(String::as_str) == Some("memory");
            if cli {
                memory = true;
                true
            } else {
                ["cd", "cat", "echo", "printf"].contains(&name.as_str())
            }
        })
        && memory;
    let mut runs = segments.iter().any(sql_runner);
    let mut programs = Vec::<String>::new();
    let mut i = 0;
    while i < segments.len() {
        if segments.len() > 100000 {
            return Err(RuntimeError::policy(
                "command too complex - nesting too deep to check",
            ));
        }
        let segment = segments[i].clone();
        let (start, mut name, mut argument) = program(&segment.words);
        programs.push(name.clone());
        let mut nested = vec![];
        if let Some(category) = generic(&segment.words, &name, argument, &mut nested) {
            return Ok(Some(category));
        }
        if !SEARCH.contains(&name.as_str()) && !["echo", "printf"].contains(&name.as_str()) {
            nested.extend(shell_commands(&segment, start, &name));
        }
        if let Some(category) = policy.apply(&segment, &name, argument, start) {
            return Ok(Some(category));
        }
        if !known(&name, policy) {
            for j in start + 1..segment.words.len() {
                let candidate = base(&segment.words[j]).tail_matches(".phar");
                let candidate = if candidate == "wp-cli" {
                    "wp"
                } else {
                    candidate
                };
                if known(candidate, policy) {
                    name = candidate.into();
                    argument = j + 1;
                    if let Some(category) = generic(&segment.words, &name, argument, &mut nested) {
                        return Ok(Some(category));
                    }
                    if let Some(category) = policy.apply(&segment, &name, argument, j) {
                        return Ok(Some(category));
                    }
                    break;
                }
            }
        }
        if segment.depth < 8 {
            for text in nested {
                if text.is_empty() {
                    continue;
                }
                let offset = segments.len();
                for mut child in split(&text)? {
                    child.depth += segment.depth + 1;
                    child.parent = child.parent.map(|parent| parent + offset).or(Some(i));
                    runs |= sql_runner(&child);
                    segments.push(child);
                }
            }
        }
        let search_parent = segment
            .parent
            .and_then(|parent| programs.get(parent))
            .is_some_and(|name| SEARCH.contains(&name.as_str()));
        if !memory_only
            && !SEARCH.contains(&name.as_str())
            && !(["cat", "echo", "printf"].contains(&name.as_str()) && search_parent)
            && let Some(category) = sql(
                &format!("{}{}", segment.words.join("\u{1f}"), segment.document),
                runs,
            )?
        {
            return Ok(Some(category));
        }
        i += 1;
    }
    Ok(None)
}
pub fn polling(command: &str) -> bool {
    !command.contains('\n')&&!command.contains("||")&&Regex::new(r"^(sleep [0-9.]+[smhd]? *(&&|;) *)?(git status|gh pr (checks|status|view)|gh run (list|view|watch)|docker (compose )?(ps|logs)|kubectl (get|describe|logs)|tail)( [^;&\x60$()<>|]*)?(\| *(head|tail|wc|cut|grep|jq)( [^;&\x60$()<>|]*)?)*$").is_ok_and(|pattern|pattern.is_match(command))
}
