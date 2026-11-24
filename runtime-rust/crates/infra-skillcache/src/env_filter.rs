//! The environment a capsule's program starts with.
//!
//! A program inherits the environment of the tool that starts it, minus every
//! variable whose name suggests a credential, plus the four variables of the
//! program contract. The filter applies to programs only: the provider's own
//! process keeps the environment the session gave it.
use std::{collections::BTreeMap, path::Path};

pub const INPUT: &str = "SKILLCACHE_INPUT";
pub const WORKSPACE: &str = "SKILLCACHE_WORKSPACE";
pub const PACKAGE: &str = "SKILLCACHE_PACKAGE";
pub const ASSETS: &str = "SKILLCACHE_ASSETS";
/// The executor's own variables; a program never sees them.
pub const STATE_FILE: &str = "SKILLCACHE_STATE_FILE";
pub const LOG_FILE: &str = "SKILLCACHE_LOG_FILE";
pub const GENERATION: &str = "SKILLCACHE_GENERATION";

/// Whether a variable is withheld from programs: its name contains a
/// credential, session, connection-string, or secret-store pattern
/// (`KEY`, `TOKEN`, `PASSWORD`, `SECRET`, `CREDENTIAL`, `PASSWD`, `AUTH`,
/// `DATABASE_URL`, `_URL`, `_URI`, `DSN`, `PGPASS`, `JWT`, `BEARER`, `COOKIE`,
/// `PRIVATE_KEY`, `KUBECONFIG`, `NETRC`, `DOCKER_CONFIG`) in any letter case.
pub fn withheld(name: &str) -> bool {
    let name = name.to_uppercase();
    [
        "KEY",
        "TOKEN",
        "PASSWORD",
        "SECRET",
        "CREDENTIAL",
        "PASSWD",
        "AUTH",
        "DATABASE_URL",
        "_URL",
        "_URI",
        "DSN",
        "PGPASS",
        "JWT",
        "BEARER",
        "COOKIE",
        "PRIVATE_KEY",
        "KUBECONFIG",
        "NETRC",
        "DOCKER_CONFIG",
    ]
    .iter()
    .any(|part| name.contains(part))
}

/// The complete environment of one program run. `inherited` is the starting
/// tool's own environment; entries that are not UTF-8 are not passed on.
pub fn program_environment(
    inherited: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    input: &str,
    workspace: &Path,
    package: &Path,
) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (name, value) in inherited {
        let (Some(name), Some(value)) = (name.to_str(), value.to_str()) else {
            continue;
        };
        if withheld(name) || name.to_uppercase().starts_with("SKILLCACHE_") {
            continue;
        }
        environment.insert(name.to_string(), value.to_string());
    }
    environment.insert(INPUT.into(), input.to_string());
    environment.insert(WORKSPACE.into(), workspace.display().to_string());
    environment.insert(PACKAGE.into(), package.display().to_string());
    environment.insert(ASSETS.into(), package.join("assets").display().to_string());
    environment
}

/// Takes `PATH` out of an environment and returns its value. On Windows the
/// name is matched in any letter case and every spelling of it is removed,
/// so that a `Path` left behind cannot shadow the `PATH` the caller writes
/// next. Elsewhere only `PATH` is that variable.
pub fn take_path(environment: &mut BTreeMap<String, String>) -> Option<String> {
    let names = environment
        .keys()
        .filter(|name| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case("PATH")
            } else {
                name.as_str() == "PATH"
            }
        })
        .cloned()
        .collect::<Vec<_>>();
    // All of them are removed before one is chosen; the names are in order,
    // and the fully capitalised spelling is the first of them.
    names
        .into_iter()
        .filter_map(|name| environment.remove(&name))
        .collect::<Vec<_>>()
        .into_iter()
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn take_path_removes_the_variable_and_on_windows_every_spelling_of_it() {
        let mut environment = BTreeMap::from([
            ("PATH".to_string(), "/bin".to_string()),
            ("Path".to_string(), "other".to_string()),
            ("LANG".to_string(), "C".to_string()),
        ]);
        assert_eq!(take_path(&mut environment).as_deref(), Some("/bin"));
        assert!(!environment.contains_key("PATH"));
        // Elsewhere `Path` is another variable and stays; on Windows it is
        // the same one and goes with it.
        assert_eq!(environment.contains_key("Path"), !cfg!(windows));
        assert!(environment.contains_key("LANG"));
        assert_eq!(take_path(&mut BTreeMap::new()), None);
        let mut other = BTreeMap::from([("Path".to_string(), "bin".to_string())]);
        assert_eq!(
            take_path(&mut other).as_deref(),
            cfg!(windows).then_some("bin")
        );
        assert_eq!(other.is_empty(), cfg!(windows));
    }

    fn inherited(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    #[test]
    fn env_filter_strips_key_token_password_case_insensitive() {
        let environment = program_environment(
            inherited(&[
                ("PATH", "/usr/bin"),
                ("HOME", "/home/dev"),
                ("TMPDIR", "/tmp/x"),
                ("LANG", "C.UTF-8"),
                ("ANTHROPIC_API_KEY", "a"),
                ("OPENAI_API_KEY", "b"),
                ("GITHUB_TOKEN", "c"),
                ("github_token", "d"),
                ("Db_Password", "e"),
                ("npm_config_authToken", "f"),
                ("SSH_KEYFILE", "g"),
                ("AWS_SECRET_VAL", "h"),
                ("GOOGLE_APPLICATION_CREDENTIALS", "i"),
                ("SUDO_PASSWD", "j"),
                ("basic_auth_header", "k"),
                ("SKILLCACHE_STATE_FILE", "/state/plan-state.json"),
                ("SKILLCACHE_GENERATION", "gen"),
                ("skillcache_input", "stale"),
            ]),
            r#"{"input_dir":"invoices/"}"#,
            Path::new("/work"),
            Path::new("/pkg"),
        );
        let names = environment.keys().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "HOME",
                "LANG",
                "PATH",
                "SKILLCACHE_ASSETS",
                "SKILLCACHE_INPUT",
                "SKILLCACHE_PACKAGE",
                "SKILLCACHE_WORKSPACE",
                "TMPDIR",
            ]
        );
        assert_eq!(environment[INPUT], r#"{"input_dir":"invoices/"}"#);
        assert_eq!(environment[WORKSPACE], "/work");
        assert_eq!(environment[PACKAGE], "/pkg");
        assert!(environment[ASSETS].ends_with("assets"));
    }

    #[test]
    fn the_platform_variables_a_program_needs_pass_the_filter() {
        for name in [
            "PATH",
            "Path",
            "HOME",
            "USERPROFILE",
            "SYSTEMROOT",
            "SystemRoot",
            "COMSPEC",
            "PATHEXT",
            "TMPDIR",
            "TMP",
            "TEMP",
            "SECURITY_MODE",
            "DURING_BUILD",
        ] {
            assert!(!withheld(name), "{name}");
        }
        for name in [
            "API_KEY",
            "apiKey",
            "X_TOKEN_Y",
            "PGPASSWORD",
            "MONKEY",
            "APP_SECRET",
            "app_secret",
            "CLOUD_CREDENTIAL",
            "ROOT_PASSWD",
            "OAUTH_HEADER",
            "basic_auth",
            "DATABASE_URL",
            "test_database_url",
            "REDIS_URL",
            "POSTGRES_URL",
            "MYSQL_URL",
            "MONGO_URL",
            "AMQP_URL",
            "WEBHOOK_URL",
            "PIP_INDEX_URL",
            "REDIS_URI",
            "mongo_uri",
            "SENTRY_DSN",
            "db_dsn",
            "PGPASS",
            "PGPASSFILE",
            "JWT",
            "supabase_jwt_val",
            "HTTP_BEARER",
            "session_cookie",
            "PRIVATE_KEY",
            "KUBECONFIG",
            "NETRC",
            "CURL_NETRC",
            "DOCKER_CONFIG",
        ] {
            assert!(withheld(name), "{name}");
        }
    }
}
