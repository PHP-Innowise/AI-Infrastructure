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

/// Whether a variable is withheld from programs: its name contains `KEY`,
/// `TOKEN` or `PASSWORD` in any letter case.
pub fn withheld(name: &str) -> bool {
    let name = name.to_uppercase();
    ["KEY", "TOKEN", "PASSWORD"]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

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
        ] {
            assert!(!withheld(name), "{name}");
        }
        for name in ["API_KEY", "apiKey", "X_TOKEN_Y", "PGPASSWORD", "MONKEY"] {
            assert!(withheld(name), "{name}");
        }
    }
}
