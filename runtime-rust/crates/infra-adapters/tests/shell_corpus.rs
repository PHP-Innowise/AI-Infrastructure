//! Runs the frozen guard corpus as data; no represented command is executed.
use infra_adapters::shell::{Policy, validate};
use serde::Deserialize;

const CORPUS: &str = include_str!("../../../../tests/fixtures/bash-validator-corpus.json");
const FRAMEWORKS: [&str; 5] = [
    "laravel",
    "symfony",
    "php-core",
    "wordpress",
    "infrastructure-creator",
];

#[derive(Deserialize)]
struct Corpus {
    editions: Vec<String>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    command: String,
    expect: Expectation,
    editions: Editions,
    note: String,
    category: Option<String>,
}
#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Expectation {
    Pass,
    Block,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Editions {
    All(String),
    Named(Vec<String>),
}

impl Case {
    fn blocks(&self, framework: &str) -> bool {
        let selected = match &self.editions {
            Editions::All(name) => {
                assert_eq!(name, "all");
                true
            }
            Editions::Named(names) => names.iter().any(|name| name == framework),
        };
        selected == (self.expect == Expectation::Block)
    }
}

struct RetiredMemoryLauncher {
    case_index: usize,
    legacy: &'static str,
    native: &'static str,
}
// These four exact pass cases refer to the removed Python entrypoint. The
// migration changes only their executable prefix. Their original strings are
// checked separately below, and no other corpus command is changed or skipped.
const RETIRED_MEMORY_LAUNCHERS: [RetiredMemoryLauncher; 4] = [
    RetiredMemoryLauncher {
        case_index: 107,
        legacy: r###"cd /srv/shop && echo -E '{"progress":"The down() migration runs DROP TABLE invoices.","verified":true,"learnings":[]}' | python3 memory-bank/scripts/context.py --root /srv/shop native-result --request 0f4c2a6b8d0e1f3a5c7e9b1d3f5a7c9e0f2b4d6a8c0e1f3a5b7d9f1a3c5e7b9d --token 6f1d3b5a7c9e0f2a4c6e8b0d2f4a6c8e --input - --json"###,
        native: r###"cd /srv/shop && echo -E '{"progress":"The down() migration runs DROP TABLE invoices.","verified":true,"learnings":[]}' | infra-engine memory --root /srv/shop native-result --request 0f4c2a6b8d0e1f3a5c7e9b1d3f5a7c9e0f2b4d6a8c0e1f3a5b7d9f1a3c5e7b9d --token 6f1d3b5a7c9e0f2a4c6e8b0d2f4a6c8e --input - --json"###,
    },
    RetiredMemoryLauncher {
        case_index: 108,
        legacy: r###"printf '%s' '{"progress":"TRUNCATE TABLE invoices resets the counters."}' | (cd /srv/shop && python3 memory-bank/scripts/context.py --root /srv/shop native-result --input - --json)"###,
        native: r###"printf '%s' '{"progress":"TRUNCATE TABLE invoices resets the counters."}' | (cd /srv/shop && infra-engine memory --root /srv/shop native-result --input - --json)"###,
    },
    RetiredMemoryLauncher {
        case_index: 109,
        legacy: r###"(cd /srv/shop && python3 memory-bank/scripts/context.py --root /srv/shop native-result --input - --json) <<'JSON'
{"progress":"The rollback runs DROP SCHEMA reports."}
JSON"###,
        native: r###"(cd /srv/shop && infra-engine memory --root /srv/shop native-result --input - --json) <<'JSON'
{"progress":"The rollback runs DROP SCHEMA reports."}
JSON"###,
    },
    RetiredMemoryLauncher {
        case_index: 110,
        legacy: r###"python3 memory-bank/scripts/context.py retrieve --query 'why does the rollback DROP TABLE invoices'"###,
        native: r###"infra-engine memory retrieve --query 'why does the rollback DROP TABLE invoices'"###,
    },
];

fn category(command: &str, policy: &Policy) -> Option<String> {
    match validate(command, policy) {
        Ok(category) => category,
        Err(error) => Some(error.message),
    }
}

#[test]
fn frozen_shell_corpus_matches_all_five_framework_scopes() {
    let corpus: Corpus = serde_json::from_str(CORPUS).expect("frozen corpus JSON");
    assert_eq!(corpus.cases.len(), 331, "review corpus changes explicitly");
    assert_eq!(corpus.editions, FRAMEWORKS);
    let mut failures = Vec::new();
    for framework in FRAMEWORKS {
        let policy = Policy::for_framework(framework);
        for (index, case) in corpus.cases.iter().enumerate() {
            let retired = RETIRED_MEMORY_LAUNCHERS
                .iter()
                .find(|entry| entry.case_index == index);
            let command = if let Some(retired) = retired {
                assert_eq!(
                    case.command, retired.legacy,
                    "retired launcher case changed"
                );
                assert!(case.expect == Expectation::Pass);
                retired.native
            } else {
                &case.command
            };
            let actual = category(command, &policy);
            if actual.is_some() != case.blocks(framework) {
                failures.push(format!(
                    "case {index}, {framework}: decision differs ({})",
                    case.note
                ));
            } else if let (Some(expected), Some(actual)) = (&case.category, actual)
                && !actual.starts_with(expected)
            {
                failures.push(format!(
                    "case {index}, {framework}: category differs ({})",
                    case.note
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn retired_python_memory_launchers_have_explicit_native_equivalents() {
    for framework in FRAMEWORKS {
        let policy = Policy::for_framework(framework);
        for entry in &RETIRED_MEMORY_LAUNCHERS {
            assert!(
                category(entry.legacy, &policy)
                    .is_some_and(|value| value.starts_with("destructive SQL")),
                "retired Python entrypoint must not retain the native exemption: case {}, {framework}",
                entry.case_index,
            );
            assert!(
                category(entry.native, &policy).is_none(),
                "native memory data exemption: case {}, {framework}",
                entry.case_index,
            );
        }
    }
}

#[test]
fn shell_long_options_do_not_consume_short_option_values() {
    let policy = Policy::for_framework("php-core");
    for command in [
        "bash --noprofile --norc -eo pipefail -c 'git reset --hard'",
        "bash --noprofile -c 'git reset --hard'",
        "bash --posix -c 'git reset --hard'",
    ] {
        assert_eq!(
            category(command, &policy).as_deref(),
            Some("destructive command - git reset --hard")
        );
    }
    assert!(
        category(
            "bash --noprofile --norc -eo pipefail -c 'git status'",
            &policy
        )
        .is_none()
    );
}

#[test]
fn artisan_db_is_a_sql_runner_for_supplied_sql_data() {
    for framework in FRAMEWORKS {
        let policy = Policy::for_framework(framework);
        for command in [
            "php artisan db <<'SQL'\nDELETE FROM users;\nSQL",
            "printf '%s' 'TRUNCATE users;' | php artisan db",
        ] {
            assert!(
                category(command, &policy)
                    .is_some_and(|value| value.starts_with("destructive SQL")),
                "artisan db SQL classification: {framework}",
            );
        }
        assert!(category("php artisan db <<'SQL'\nSELECT 1;\nSQL", &policy).is_none());
        assert!(category("echo 'delete from the cart'", &policy).is_none());
    }
}

#[test]
fn launcher_normalizes_wp_cli_before_policy_lookup() {
    for framework in FRAMEWORKS {
        let policy = Policy::for_framework(framework);
        let blocks = matches!(framework, "wordpress" | "infrastructure-creator");
        for command in [
            "ddev wp-cli db reset --yes",
            "docker compose exec app wp-cli.phar db reset --yes",
        ] {
            assert_eq!(
                category(command, &policy).as_deref(),
                blocks.then_some("destructive command - wp db reset"),
                "launcher policy scope: {framework}",
            );
        }
        assert!(category("ddev wp-cli db tables", &policy).is_none());
    }
}

#[test]
fn encoded_shell_children_are_visible_to_sql_checks_before_parent_data() {
    let cases = [
        (
            "nested pipeline",
            r###"bash -c "printf '%s' 'DELETE FROM users;' | php artisan db""###,
            true,
        ),
        (
            "outer heredoc DELETE",
            "bash -c \"php artisan db\" <<'SQL'\nDELETE FROM users;\nSQL",
            true,
        ),
        (
            "outer heredoc bare TRUNCATE",
            "bash -c \"php artisan db\" <<'SQL'\nTRUNCATE users;\nSQL",
            true,
        ),
        (
            "quoted SQL without runner",
            r###"bash -c "printf '%s' 'DELETE FROM users;'""###,
            false,
        ),
        (
            "quoted runner name is data",
            r###"bash -c "printf '%s' 'DELETE FROM users;' 'php artisan db'""###,
            false,
        ),
        (
            "filtered DELETE",
            "bash -c \"php artisan db\" <<'SQL'\nDELETE FROM users WHERE id = 1;\nSQL",
            false,
        ),
        (
            "harmless query",
            "bash -c \"php artisan db\" <<'SQL'\nSELECT 1;\nSQL",
            false,
        ),
    ];
    let mut failures = Vec::new();
    for framework in FRAMEWORKS {
        let policy = Policy::for_framework(framework);
        for (label, command, blocked) in cases {
            let actual = category(command, &policy);
            if actual.is_some() != blocked
                || blocked
                    && !actual
                        .as_deref()
                        .is_some_and(|value| value.starts_with("destructive SQL"))
            {
                failures.push(format!("{label}, {framework}: SQL decision differs"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
