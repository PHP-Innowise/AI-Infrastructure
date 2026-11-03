use crate::common::*;
use infra_core::{Result, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::json as object;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
const READONLY: &[&str] = &[
    "@putenv",
    "ava",
    "behat",
    "cat",
    "date",
    "deptrac",
    "diff",
    "echo",
    "eslint",
    "false",
    "find",
    "flake8",
    "gofmt",
    "grep",
    "head",
    "jest",
    "ls",
    "mocha",
    "mypy",
    "parallel-lint",
    "paratest",
    "pest",
    "phpcs",
    "phploc",
    "phpmd",
    "phpspec",
    "phpstan",
    "phpunit",
    "printf",
    "psalm",
    "pwd",
    "pylint",
    "pytest",
    "sort",
    "stylelint",
    "tail",
    "test",
    "true",
    "tsc",
    "uniq",
    "unittest",
    "vitest",
    "wc",
    "which",
];
const FORMATTERS: &[&str] = &["black", "php-cs-fixer", "pint", "prettier", "ruff"];
const SHELLS: &[&str] = &["bash", "dash", "fish", "sh", "zsh"];
const INTERPRETERS: &[&str] = &["node", "php", "python", "python3"];
const PACKAGE: &[&str] = &["npm", "pnpm", "yarn", "bun", "composer"];
const NETWORK: &[&str] = &[
    "curl", "wget", "ssh", "scp", "sftp", "rsync", "nc", "ncat", "telnet",
];
const PROVIDERS: &[&str] = &[
    "aws",
    "az",
    "gcloud",
    "heroku",
    "stripe",
    "shopify",
    "firebase",
    "gh",
    "vercel",
    "netlify",
    "openai",
    "kubectl",
    "helm",
    "terraform",
    "tofu",
    "pulumi",
];
const FS_REMOVE: &[&str] = &["rm", "rmdir", "shred", "unlink"];
const FS_WRITE: &[&str] = &["cp", "install", "ln", "mkdir", "mv", "touch"];
const WRAPPERS: &[&str] = &["env", "nice", "nohup", "stdbuf", "sudo", "timeout"];
const MUTATE: &[&str] = &[
    "--fix",
    "--fix-dry-run=false",
    "--write",
    "-w",
    "--apply",
    "--update",
    "--alter",
    "--generate-baseline",
    "--set-baseline",
    "--update-baseline",
    "--migrate-configuration",
];
const DEPENDENCIES: &[&str] = &[
    "add",
    "create",
    "create-project",
    "install",
    "publish",
    "remove",
    "require",
    "self-update",
    "update",
];
const COMPOSER_BUILTINS: &[&str] = &[
    "about",
    "archive",
    "audit",
    "browse",
    "check-platform-reqs",
    "clear-cache",
    "config",
    "create-project",
    "depends",
    "diagnose",
    "dump-autoload",
    "exec",
    "fund",
    "global",
    "help",
    "init",
    "install",
    "licenses",
    "list",
    "outdated",
    "prohibits",
    "reinstall",
    "remove",
    "repository",
    "require",
    "run-script",
    "search",
    "self-update",
    "show",
    "status",
    "suggests",
    "update",
    "validate",
];
const PACKAGE_BUILTINS: &[&str] = &[
    "add",
    "audit",
    "bin",
    "cache",
    "ci",
    "config",
    "create",
    "dedupe",
    "dlx",
    "doctor",
    "exec",
    "fetch",
    "global",
    "help",
    "i",
    "import",
    "info",
    "init",
    "install",
    "licenses",
    "link",
    "list",
    "ls",
    "node",
    "outdated",
    "pack",
    "patch",
    "patch-commit",
    "prune",
    "publish",
    "rebuild",
    "remove",
    "rm",
    "run",
    "run-script",
    "setup",
    "store",
    "un",
    "uninstall",
    "unlink",
    "up",
    "update",
    "upgrade",
    "version",
    "versions",
    "why",
    "workspace",
    "workspaces",
    "x",
];
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub category: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandAnalysis {
    pub command: String,
    pub categories: Vec<String>,
    pub findings: Vec<Finding>,
    pub expanded_commands: Vec<String>,
    pub aliases: Vec<String>,
    pub verification_safe: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetAnalysis {
    pub target: String,
    pub scripts: BTreeMap<String, CommandAnalysis>,
    pub commands: Vec<CommandAnalysis>,
}
#[derive(Default)]
struct Accumulator {
    findings: Vec<Finding>,
    expanded: Vec<String>,
    aliases: Vec<String>,
    count: usize,
}
impl Accumulator {
    fn add(&mut self, code: &str, category: &str, message: impl Into<String>) {
        let item = Finding {
            code: code.into(),
            category: category.into(),
            message: message.into(),
        };
        if !self.findings.contains(&item) {
            self.findings.push(item);
        }
    }
    fn block(&mut self, code: &str, message: impl Into<String>) {
        self.add(code, "verification_blocker", message);
    }
}
fn basename(value: &str) -> String {
    value
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase()
}
pub fn known_executable(value: &str) -> bool {
    let value = basename(value);
    [
        READONLY,
        FORMATTERS,
        SHELLS,
        INTERPRETERS,
        PACKAGE,
        NETWORK,
        PROVIDERS,
        FS_REMOVE,
        FS_WRITE,
        WRAPPERS,
        &[
            "infra-engine",
            "infra-engine.exe",
            "bunx",
            "npx",
            "artisan",
            "cap",
            "console",
            "deploy",
            "deployer",
            "git",
            "mysql",
            "psql",
            "symfony",
        ],
    ]
    .iter()
    .any(|set| set.contains(&value.as_str()))
}
pub fn quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}
pub fn tokenize(value: &str) -> Result<Vec<String>> {
    let mut result = vec![];
    let mut token = String::new();
    let mut quote = None;
    let mut active = false;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    token.push(character);
                }
            }
            Some('"') => {
                if character == '"' {
                    quote = None;
                } else if character == '\\' {
                    let next = characters
                        .next()
                        .ok_or_else(|| RuntimeError::invalid("No escaped character"))?;
                    if !matches!(next, '"' | '\\' | '$' | '\n') && next as u32 != 96 {
                        token.push('\\');
                    }
                    if next != '\n' {
                        token.push(next);
                    }
                } else {
                    token.push(character);
                }
            }
            _ => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    active = true;
                }
                '\\' => {
                    token.push(
                        characters
                            .next()
                            .ok_or_else(|| RuntimeError::invalid("No escaped character"))?,
                    );
                    active = true;
                }
                character if character.is_whitespace() => {
                    if active {
                        result.push(std::mem::take(&mut token));
                        active = false;
                    }
                }
                _ => {
                    token.push(character);
                    active = true;
                }
            },
        }
    }
    if quote.is_some() {
        return Err(RuntimeError::invalid("No closing quotation"));
    }
    if active {
        result.push(token);
    }
    Ok(result)
}
pub struct CommandAnalyzer {
    pub target: PathBuf,
    pub scripts: BTreeMap<(String, String), Vec<String>>,
}
impl CommandAnalyzer {
    pub fn new(target: &Path) -> Result<Self> {
        let target = crate::ownership::resolve_target(target)?;
        if !target.is_dir() {
            return Err(RuntimeError::invalid("target is not a directory"));
        }
        let mut scripts = BTreeMap::new();
        for (filename, ecosystem) in [("composer.json", "composer"), ("package.json", "package")] {
            let path = target.join(filename);
            if !path.is_file() {
                continue;
            }
            let document = read_object(&path)?;
            let entries = document.get("scripts").cloned().unwrap_or(object!({}));
            let entries = entries
                .as_object()
                .ok_or_else(|| RuntimeError::invalid("scripts must be an object"))?;
            for (name, value) in entries {
                if name.is_empty() {
                    return Err(RuntimeError::invalid(
                        "script names must be non-empty strings",
                    ));
                }
                let commands = if let Some(command) = value.as_str() {
                    vec![command.into()]
                } else if ecosystem == "composer" && value.is_array() {
                    if !value
                        .as_array()
                        .is_some_and(|items| items.iter().all(serde_json::Value::is_string))
                    {
                        return Err(RuntimeError::invalid(
                            "Composer script list must contain strings",
                        ));
                    }
                    texts(value)
                } else if ecosystem == "composer" && value.is_object() {
                    value
                        .as_object()
                        .ok_or_else(|| RuntimeError::invalid("Script object missing"))?
                        .iter()
                        .map(|(command, handler)| match handler.as_str() {
                            Some("symfony-cmd") => Ok(format!("bin/console {command}")),
                            Some("php-script") => Ok(format!("php {command}")),
                            Some(_) => Ok(command.clone()),
                            None => Err(RuntimeError::invalid("Flex handler must be a string")),
                        })
                        .collect::<Result<Vec<_>>>()?
                } else {
                    return Err(RuntimeError::invalid(format!(
                        "{filename}: invalid script {name}"
                    )));
                };
                scripts.insert((ecosystem.into(), name.clone()), commands);
            }
        }
        Ok(Self { target, scripts })
    }
    pub fn analyze(
        &self,
        command: &str,
        verification: bool,
        ecosystem: Option<&str>,
    ) -> Result<CommandAnalysis> {
        if command.trim().is_empty() {
            return Err(RuntimeError::invalid("command must be non-empty"));
        }
        if ecosystem.is_some_and(|value| !["composer", "package"].contains(&value)) {
            return Err(RuntimeError::invalid("Unknown script ecosystem"));
        }
        let mut acc = Accumulator::default();
        self.walk(command, ecosystem, &[], &mut acc);
        let order = [
            "non_mutating",
            "workspace_mutation",
            "destructive_database_deploy",
            "external_provider_network",
        ];
        let mut categories = order
            .into_iter()
            .filter(|category| acc.findings.iter().any(|item| item.category == *category))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let blocked = acc
            .findings
            .iter()
            .any(|item| item.category == "verification_blocker");
        if categories.is_empty() && !blocked {
            categories.push("non_mutating".into());
        }
        let verification_safe =
            !blocked && categories.len() == 1 && categories[0] == "non_mutating";
        if blocked {
            categories.push("verification_blocker".into());
        }
        if verification && !verification_safe {
            acc.block("UNSAFE_VERIFICATION","verification commands must be non-mutating, local, composition-free, and fully resolved");
        }
        Ok(CommandAnalysis {
            command: command.into(),
            categories,
            findings: acc.findings,
            expanded_commands: acc.expanded,
            aliases: acc.aliases,
            verification_safe,
        })
    }
    pub fn analyze_target(
        &self,
        commands: &[String],
        verification: bool,
        no_scripts: bool,
    ) -> Result<TargetAnalysis> {
        let mut scripts = BTreeMap::new();
        if !no_scripts {
            for (ecosystem, name) in self.scripts.keys() {
                let invocation = if ecosystem == "composer" {
                    format!("composer run-script {}", quote(name))
                } else {
                    format!("npm run {}", quote(name))
                };
                scripts.insert(
                    format!("{ecosystem}:{name}"),
                    self.analyze(&invocation, verification, None)?,
                );
            }
        }
        Ok(TargetAnalysis {
            target: path_text(&self.target)?,
            scripts,
            commands: commands
                .iter()
                .map(|command| self.analyze(command, verification, None))
                .collect::<Result<Vec<_>>>()?,
        })
    }
    fn resolve_alias(
        &self,
        tokens: &[String],
        ecosystem: Option<&str>,
    ) -> Option<((String, String), Vec<String>)> {
        let first = basename(tokens.first()?);
        if first.starts_with('@')
            && ecosystem == Some("composer")
            && !["@php", "@composer", "@putenv"].contains(&first.as_str())
        {
            return Some((("composer".into(), first[1..].into()), tokens[1..].to_vec()));
        }
        if !PACKAGE.contains(&first.as_str()) {
            return None;
        }
        let index = (1..tokens.len()).find(|index| !tokens[*index].starts_with('-'))?;
        let action = &tokens[index];
        if ["run", "run-script"].contains(&action.as_str()) {
            let alias = (index + 1..tokens.len()).find(|index| !tokens[*index].starts_with('-'));
            return Some((
                (
                    if first == "composer" {
                        "composer"
                    } else {
                        "package"
                    }
                    .into(),
                    alias.map(|index| tokens[index].clone()).unwrap_or_default(),
                ),
                alias
                    .map(|index| tokens[index + 1..].to_vec())
                    .unwrap_or_default(),
            ));
        }
        if first == "composer" {
            if COMPOSER_BUILTINS.contains(&action.as_str()) {
                return None;
            }
            return Some((
                ("composer".into(), action.clone()),
                tokens[index + 1..].to_vec(),
            ));
        }
        if ["test", "start", "stop", "restart"].contains(&action.as_str())
            || (["pnpm", "yarn", "bun"].contains(&first.as_str())
                && !PACKAGE_BUILTINS.contains(&action.as_str()))
        {
            Some((
                ("package".into(), action.clone()),
                tokens[index + 1..].to_vec(),
            ))
        } else {
            None
        }
    }
    fn walk(
        &self,
        command: &str,
        ecosystem: Option<&str>,
        stack: &[(String, String)],
        acc: &mut Accumulator,
    ) {
        if acc.count >= 512 {
            acc.block("EXPANSION_LIMIT", "alias expansion exceeded 512 commands");
            return;
        }
        acc.count += 1;
        if matching(r"(?:&&|\|\||[;&|<>\x60]|\$\(|[\r\n])", command) {
            acc.block(
                "SHELL_COMPOSITION",
                "shell composition cannot be analyzed safely",
            );
            return;
        }
        let tokens = match tokenize(command) {
            Ok(tokens) if !tokens.is_empty() => tokens,
            Ok(_) => {
                acc.block("TOKENIZE_ERROR", "empty command");
                return;
            }
            Err(error) => {
                acc.block("TOKENIZE_ERROR", error.to_string());
                return;
            }
        };
        if let Some((key, trailing)) = self.resolve_alias(&tokens, ecosystem) {
            let label = format!("{}:{}", key.0, key.1);
            acc.aliases.push(label.clone());
            if stack.contains(&key) {
                acc.block(
                    "ALIAS_CYCLE",
                    format!("script alias cycle detected: {label}"),
                );
                return;
            }
            let Some(bodies) = self.scripts.get(&key) else {
                acc.block(
                    "UNKNOWN_ALIAS",
                    format!("script alias {label:?} is not defined"),
                );
                return;
            };
            let mut chain = stack.to_vec();
            chain.push(key.clone());
            if key.0 == "package" {
                let pre = format!("pre{}", key.1);
                if self.scripts.contains_key(&("package".into(), pre.clone())) {
                    self.walk(
                        &format!("npm run {}", quote(&pre)),
                        Some("package"),
                        &chain,
                        acc,
                    );
                }
            }
            for body in bodies {
                let expanded = if trailing.is_empty() {
                    body.clone()
                } else {
                    format!(
                        "{body} {}",
                        trailing
                            .iter()
                            .map(|value| quote(value))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                };
                self.walk(&expanded, Some(&key.0), &chain, acc);
            }
            if key.0 == "package" {
                let post = format!("post{}", key.1);
                if self.scripts.contains_key(&("package".into(), post.clone())) {
                    self.walk(
                        &format!("npm run {}", quote(&post)),
                        Some("package"),
                        &chain,
                        acc,
                    );
                }
            }
            return;
        }
        acc.expanded.push(command.into());
        self.leaf(tokens, command, acc);
    }
    fn leaf(&self, mut working: Vec<String>, command: &str, acc: &mut Accumulator) {
        while !working.is_empty() {
            if matching(r"^[A-Za-z_][A-Za-z0-9_]*=.*$", &working[0]) {
                working.remove(0);
                continue;
            }
            let wrapper = basename(&working[0]);
            if !WRAPPERS.contains(&wrapper.as_str()) {
                break;
            }
            if wrapper == "sudo" {
                acc.block("SUDO_EXECUTION", "command escalates privileges with sudo");
            }
            working.remove(0);
            let values: &[&str] = match wrapper.as_str() {
                "env" => &["-C", "-S", "-u", "--chdir", "--split-string", "--unset"],
                "nice" => &["-n", "--adjustment"],
                "stdbuf" => &["-e", "-i", "-o", "--error", "--input", "--output"],
                "sudo" => &["-g", "--group", "-p", "--prompt", "-u", "--user"],
                "timeout" => &["-k", "--kill-after", "-s", "--signal"],
                _ => &[],
            };
            while working.first().is_some_and(|value| value.starts_with('-')) {
                let flag = working.remove(0);
                if values.contains(&flag.as_str()) && !working.is_empty() {
                    working.remove(0);
                }
            }
            if wrapper == "timeout" && !working.is_empty() {
                working.remove(0);
            }
        }
        if working.is_empty() {
            acc.block(
                "TOKENIZE_ERROR",
                "command contains no executable after environment assignments",
            );
            return;
        }
        if working[0] == "@php" {
            working[0] = "php".into();
        }
        if working[0] == "@composer" {
            working[0] = "composer".into();
        }
        if working[0].starts_with('$') || basename(&working[0]) == "xargs" {
            acc.block(
                "COMMAND_INDIRECTION",
                "command resolves its executable indirectly",
            );
            return;
        }
        let executable = basename(&working[0]);
        let args = &working[1..];
        let lower = args
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let has = |set: &[&str]| args.iter().any(|value| set.contains(&value.as_str()));
        let action = lower
            .iter()
            .find(|value| !value.starts_with('-'))
            .map(String::as_str)
            .unwrap_or("");
        if !known_executable(&executable) {
            acc.block(
                "UNKNOWN_EXECUTABLE",
                format!("executable {executable:?} is not a known read-only tool"),
            );
        }
        if INTERPRETERS.contains(&executable.as_str())
            && self.interpreter(&executable, args, command, acc)
        {
            return;
        }
        if ["infra-engine", "infra-engine.exe"].contains(&executable.as_str()) {
            let attested = crate::assets::runtime_contract()["commands"]["read_health"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.as_str())
                .any(|entry| tokenize(entry).is_ok_and(|parts| parts.get(1..) == Some(args)));
            if attested {
                return;
            }
            let service = args.first().map(String::as_str).unwrap_or("");
            let operation = args.get(1).map(String::as_str).unwrap_or("");
            let memory_write = service == "memory"
                && [
                    "start",
                    "update",
                    "complete",
                    "index",
                    "compact",
                    "export",
                    "checkpoint",
                    "record-result",
                    "capture",
                    "promote",
                    "task",
                    "tasks",
                    "finding",
                    "incident",
                    "decision",
                    "handoff",
                    "session",
                    "plan",
                    "record",
                    "records",
                ]
                .contains(&operation);
            let bank_write = service == "bank"
                && [
                    "add",
                    "update",
                    "capture",
                    "promote",
                    "retire",
                    "archive",
                    "supersede",
                    "import",
                ]
                .contains(&operation);
            let continuity_write = service == "continuity";
            if memory_write || bank_write || continuity_write {
                acc.add(
                    "NATIVE_WORKSPACE_MUTATION",
                    "workspace_mutation",
                    "native runtime command changes local workflow, index, memory or export files",
                );
            } else {
                acc.block("NATIVE_COMMAND_UNATTESTED","native runtime command is not in the trusted read-health contract or known mutation grammar");
            }
            return;
        }

        if SHELLS.contains(&executable.as_str())
            && (args.iter().any(|value| {
                value == "--command"
                    || value.starts_with('-') && !value.starts_with("--") && value.contains('c')
            }) || args.iter().any(|value| !value.starts_with('-'))
                || !has(&["--help", "--version"]))
        {
            acc.block(
                "SHELL_INTERPRETER",
                "command delegates execution to an unanalyzed shell",
            );
        }
        if has(MUTATE) {
            acc.add(
                "WORKSPACE_WRITE_FLAG",
                "workspace_mutation",
                "command uses a write/fix flag",
            );
        }
        if FORMATTERS.contains(&executable.as_str())
            && action != "check"
            && !has(&["--check", "--dry-run", "--test", "--diff"])
            && (executable != "prettier" || has(&["--write"]))
        {
            acc.add(
                "FORMAT_WRITES",
                "workspace_mutation",
                "formatter defaults to writing without check or dry-run mode",
            );
        }
        if executable == "git" {
            let mut at = 0;
            while at < args.len() && args[at].starts_with('-') {
                at += if ["-C", "-c", "--git-dir", "--work-tree"].contains(&args[at].as_str()) {
                    2
                } else {
                    1
                };
            }
            let action = args.get(at).map(String::as_str).unwrap_or("");
            if ["reset", "clean"].contains(&action) {
                acc.add(
                    "DESTRUCTIVE_GIT",
                    "destructive_database_deploy",
                    "git can destroy workspace state",
                );
            } else if [
                "add", "commit", "checkout", "switch", "merge", "rebase", "restore", "stash", "tag",
            ]
            .contains(&action)
            {
                acc.add(
                    "GIT_MUTATION",
                    "workspace_mutation",
                    "git command mutates repository state",
                );
            }
            if ["clone", "fetch", "pull", "push", "submodule"].contains(&action) {
                acc.add(
                    "GIT_NETWORK",
                    "external_provider_network",
                    "git command can access a remote",
                );
            }
        }
        let console = if ["artisan", "console"].contains(&executable.as_str()) {
            lower.first().map(String::as_str).unwrap_or("")
        } else if executable == "symfony" && lower.first().is_some_and(|value| value == "console") {
            lower.get(1).map(String::as_str).unwrap_or("")
        } else {
            ""
        };
        if !console.is_empty() && !console_read_only(console, &lower) {
            acc.block(
                "CONSOLE_ACTION_UNATTESTED",
                format!("console action {console:?} is not a listed read-only action"),
            );
        }
        let database = [
            "db:wipe",
            "db:seed",
            "schema:drop",
            "schema:update",
            "database:drop",
        ];
        let migration = |value: &str| value.contains("migrat") && !value.ends_with(":status");
        let provider_mutation = match executable.as_str() {
            "terraform" | "tofu" => ["apply", "destroy", "import"].contains(&action),
            "kubectl" => {
                ["apply", "create", "delete", "patch", "replace", "scale"].contains(&action)
            }
            "helm" => ["install", "rollback", "uninstall", "upgrade"].contains(&action),
            "serverless" => ["deploy", "remove", "rollback"].contains(&action),
            "sam" => action == "deploy",
            "docker" => action == "push",
            _ => false,
        };
        if ["deploy", "cap", "deployer"].contains(&executable.as_str())
            || database.contains(&action)
            || migration(action)
            || database.iter().any(|value| console.contains(value))
            || migration(console)
            || ["deploy", "release", "rollback"].contains(&console)
            || provider_mutation
            || (["mysql", "psql"].contains(&executable.as_str()) && !args.is_empty())
        {
            acc.add(
                "DESTRUCTIVE_DATABASE_DEPLOY",
                "destructive_database_deploy",
                "command can mutate database, infrastructure, or deployment state",
            );
        }
        if NETWORK.contains(&executable.as_str()) || PROVIDERS.contains(&executable.as_str()) {
            acc.add(
                "EXTERNAL_OR_PROVIDER",
                "external_provider_network",
                "command may contact external service or provider",
            );
        }
        if [
            "aws",
            "s3",
            "stripe",
            "shopify",
            "firebase",
            "openai",
            "meilisearch",
            "storyblok",
            "akeneo",
        ]
        .iter()
        .any(|value| console.contains(value))
            && ["sync", "send", "publish", "upload", "download"]
                .iter()
                .any(|value| console.contains(value))
        {
            acc.add(
                "PROVIDER_OPERATION",
                "external_provider_network",
                "console action can invoke a provider",
            );
        }
        if PACKAGE.contains(&executable.as_str()) {
            let positional = lower
                .iter()
                .filter(|value| !value.starts_with('-'))
                .collect::<Vec<_>>();
            let action = if positional
                .first()
                .is_some_and(|value| value.as_str() == "global")
            {
                positional.get(1)
            } else {
                positional.first()
            }
            .map(|value| value.as_str())
            .unwrap_or("");
            if DEPENDENCIES.contains(&action)
                || (executable != "composer" && ["ci", "i", "up", "upgrade"].contains(&action))
            {
                acc.add(
                    "DEPENDENCY_WRITE",
                    "workspace_mutation",
                    "package command changes dependencies or generated files",
                );
                acc.add(
                    "DEPENDENCY_NETWORK",
                    "external_provider_network",
                    "package command can access package registry",
                );
            }
            if executable != "composer" && ["dlx", "exec", "x"].contains(&action) {
                acc.add(
                    "DEPENDENCY_EXECUTE",
                    "workspace_mutation",
                    "package command can download and execute code",
                );
                acc.add(
                    "DEPENDENCY_NETWORK",
                    "external_provider_network",
                    "package command can access registry",
                );
            }
        }
        if ["bunx", "npx"].contains(&executable.as_str()) {
            acc.add(
                "DEPENDENCY_EXECUTE",
                "workspace_mutation",
                "package runner can download and execute code",
            );
            acc.add(
                "DEPENDENCY_NETWORK",
                "external_provider_network",
                "package runner can access registry",
            );
        }
        if executable == "find"
            && has(&[
                "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf",
                "-fls",
            ])
        {
            acc.add(
                "FIND_ACTION",
                "workspace_mutation",
                "find acts on matched paths",
            );
        }
        if FS_REMOVE.contains(&executable.as_str()) {
            acc.add(
                "DESTRUCTIVE_FILESYSTEM",
                "destructive_database_deploy",
                "command removes workspace data",
            );
        } else if FS_WRITE.contains(&executable.as_str()) {
            acc.add(
                "FILESYSTEM_MUTATION",
                "workspace_mutation",
                "command can write workspace data",
            );
        }
    }
    fn interpreter(
        &self,
        executable: &str,
        args: &[String],
        command: &str,
        acc: &mut Accumulator,
    ) -> bool {
        if executable == "php"
            && args
                .iter()
                .any(|value| ["--syntax-check", "-l"].contains(&value.as_str()))
        {
            return false;
        }
        let (inline, letters, value_flags, reports): (&[&str], &str, &[&str], &[&str]) =
            match executable {
                "node" => (
                    &[
                        "--eval",
                        "--interactive",
                        "--print",
                        "--require",
                        "-e",
                        "-i",
                        "-p",
                        "-r",
                    ],
                    "eipr",
                    &["--conditions"],
                    &["--help", "--test", "--version", "-h", "-v"],
                ),
                "php" => (
                    &["-F", "-R", "-a", "-f", "-r"],
                    "FRafr",
                    &["-c", "-d"],
                    &[
                        "--help",
                        "--ini",
                        "--modules",
                        "--version",
                        "-h",
                        "-i",
                        "-m",
                        "-v",
                    ],
                ),
                _ => (
                    &["-c", "-i"],
                    "ci",
                    &["-W", "-X"],
                    &["--help", "--version", "-V", "-h"],
                ),
            };
        let mut at = 0;
        while at < args.len() {
            let token = &args[at];
            if !token.starts_with('-') {
                if known_executable(token) {
                    self.leaf(args[at..].to_vec(), command, acc);
                } else {
                    acc.block(
                        "INTERPRETER_EXECUTION",
                        format!("command runs unanalyzed {executable} script {token:?}"),
                    );
                }
                return true;
            }
            let base = token.split('=').next().unwrap_or(token);
            if inline.contains(&base)
                || (matching(r"^-[A-Za-z]+$", token)
                    && token[1..].chars().any(|value| letters.contains(value)))
            {
                acc.block(
                    "INTERPRETER_EXECUTION",
                    "command passes inline code to interpreter",
                );
                return true;
            }
            if ["python", "python3"].contains(&executable) && base == "-m" {
                if at + 1 < args.len() {
                    self.leaf(args[at + 1..].to_vec(), command, acc);
                } else {
                    acc.block("INTERPRETER_EXECUTION", "command names no module to run");
                }
                return true;
            }
            at += if value_flags.contains(&base) && !token.contains('=') {
                2
            } else {
                1
            };
        }
        if !args.iter().any(|value| reports.contains(&value.as_str())) {
            acc.block(
                "INTERPRETER_EXECUTION",
                "command starts interactive interpreter reading stdin",
            );
            return true;
        }
        false
    }
}
pub fn console_read_only(action: &str, args: &[String]) -> bool {
    if ["--help", "--version", "-V", "-h"].contains(&action) {
        return true;
    }
    if [
        "debug:dotenv",
        "config:show",
        "env:decrypt",
        "secrets:reveal",
        "secrets:decrypt-to-local",
    ]
    .contains(&action)
        || (action == "debug:container" && args.iter().any(|value| value.starts_with("--env-var")))
    {
        return false;
    }
    [
        "about",
        "help",
        "list",
        "test",
        "route:list",
        "event:list",
        "schedule:list",
        "model:show",
        "db:show",
        "db:table",
        "queue:failed",
        "queue:monitor",
        "doctrine:schema:validate",
        "doctrine:mapping:info",
        "doctrine:migrations:list",
        "doctrine:migrations:up-to-date",
        "router:match",
        "config:dump-reference",
        "importmap:audit",
        "importmap:outdated",
        "messenger:stats",
        "messenger:failed:show",
        "cache:pool:list",
    ]
    .contains(&action)
        || action.starts_with("debug:")
        || action.starts_with("lint:")
        || action.ends_with(":status")
}
