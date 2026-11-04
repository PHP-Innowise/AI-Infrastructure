//! Guard inputs are data; no represented command or provider is executed.
use infra_adapters::{hooks, output::Output};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

const CHILD_ROOT: &str = "INFRA_HOOK_PREFILTER_CHILD_ROOT";
const HOSTS: [&str; 3] = ["claude", "codex", "cursor"];
const ROLES: [&str; 3] = ["bash-validator", "file-naming-validator", "loop-detection"];

fn run(root: &Path, host: &str, role: &str, payload: &Value) -> Output {
    hooks::execute(root, host, role, &serde_json::to_vec(payload).unwrap()).unwrap()
}

fn silent(output: Output) {
    assert_eq!(output.code, 0, "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(output.after_delivery.is_none());
}

fn exercise(root: &Path) {
    let temporary = tempfile::tempdir_in(root).unwrap();
    let project = temporary.path().join("project");
    fs::create_dir(&project).unwrap();
    let missing = temporary.path().join("missing project");
    let broken = temporary.path().join("broken checkout");
    fs::create_dir(&broken).unwrap();
    fs::write(
        broken.join(".git"),
        format!(
            "gitdir: {}\n",
            temporary.path().join("missing git directory").display()
        ),
    )
    .unwrap();
    let policy_root = temporary.path().join("invalid policy");
    fs::create_dir_all(policy_root.join("project-brain/config")).unwrap();
    fs::write(
        policy_root.join("project-brain/config/hook-policy.json"),
        "{",
    )
    .unwrap();

    // A read names a forbidden output path but has neither a command nor an
    // edit tool. Policy/context failures also keep this exact silent result.
    let read = json!({"tool_name":"Read","tool_input":{"file_path":"memory-bank/chunks/bad.md"}});
    for host in HOSTS {
        for bound in [&project, &missing, &broken, &policy_root] {
            for role in ROLES {
                silent(run(bound, host, role, &read));
            }
        }
        for raw in [
            b"".as_slice(),
            b"not json",
            b"null",
            b"false",
            b"42",
            b"\"text\"",
            b"[{\"command\":\"rm -rf /\"}]",
        ] {
            for role in ROLES {
                silent(hooks::execute(&project, host, role, raw).unwrap());
            }
        }

        // A non-shell tool label cannot hide a real recursively nested command,
        // including when Context cannot be built and raw fallback is required.
        for payload in [
            json!({"tool_name":"Read","command":"","input":[{"cmd":"git push --force origin main"}]}),
            json!({"tool_name":null,"input":{"command":false,"child":{"command":"rm -rf /"}}}),
        ] {
            for bound in [&project, &missing, &broken, &policy_root] {
                let output = run(bound, host, "bash-validator", &payload);
                assert_eq!(output.code, 2, "{host} {}: {output:?}", bound.display());
                let message = String::from_utf8(output.stderr).unwrap();
                assert!(message.contains("BLOCKED:"), "{message}");
                assert!(output.stdout.is_empty());
                if bound == &policy_root {
                    assert!(message.contains("policy data or custody is invalid"));
                }
            }
        }

        // Missing/null names remain potentially applicable; namespaced and
        // nested edit names retain the existing suffix matching contract.
        for payload in [
            json!({"tool_input":{"file_path":"memory-bank/chunks/bad.md"}}),
            json!({"tool_name":null,"tool_input":{"file_path":"memory-bank/chunks/bad.md"}}),
            json!({"tool_name":"Read","nested":[{"tool_name":"provider:Multi_Edit"}],"tool_input":{"file_path":"memory-bank/chunks/bad.md"}}),
            json!({"tool_name":"tools.apply_patch","input":"*** Begin Patch\n*** Add File: memory-bank/chunks/bad.md\n+body\n*** End Patch\n"}),
        ] {
            let output = run(&project, host, "file-naming-validator", &payload);
            assert_eq!(output.code, 2, "{host} {payload}: {output:?}");
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("BLOCKED:")
            );
        }

        // Exercise the same applicability cases through real edit counters.
        // The child has a private TMPDIR, so every counter belongs to this test.
        for (index, mut payload) in [
            json!({"tool_input":{"file_path":"src/Missing.php"}}),
            json!({"tool_name":null,"tool_input":{"file_path":"src/Null.php"}}),
            json!({"tool_name":"Read","nested":[{"tool_name":"provider:Multi_Edit"}],"tool_input":{"file_path":"src/Mixed.php"}}),
        ]
        .into_iter()
        .enumerate()
        {
            payload["session_id"] = json!(format!("prefilter-{host}-{index}"));
            for _ in 0..6 {
                silent(run(&project, host, "loop-detection", &payload));
            }
            let warning = run(&project, host, "loop-detection", &payload);
            assert_eq!(
                warning.code,
                if host == "cursor" { 1 } else { 0 },
                "{warning:?}"
            );
            assert!(
                String::from_utf8(warning.stdout)
                    .unwrap()
                    .contains("edited 7 times")
            );
        }
    }
    assert!(!project.join("project-brain").exists());
    assert!(!project.join("memory-bank").exists());
}

#[test]
fn guard_payload_prefilter_preserves_guard_contracts() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = Path::new(&root);
        assert!(root.is_absolute() && root.is_dir());
        exercise(root);
        return;
    }
    // Hook environment and counters are scoped to a child of this owned fixture.
    // HOME, PATH, credentials, provider configuration and user files are untouched.
    let temporary = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "guard_payload_prefilter_preserves_guard_contracts",
            "--nocapture",
        ])
        .env(CHILD_ROOT, temporary.path())
        .env("TMPDIR", temporary.path())
        .current_dir(temporary.path());
    for name in [
        "ACCELERATOR_HOME",
        "ACCELERATOR_STATE_DIR",
        "ACCELERATOR_PROJECT_DIR",
        "CLAUDE_PROJECT_DIR",
        "CONTEXT_TASK_ID",
        "CONTEXT_HOOK_BUDGET",
        "CONTEXT_MEMORY_RECOVERY",
        "INFRA_SOURCE_ROOT",
    ] {
        child.env_remove(name);
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
