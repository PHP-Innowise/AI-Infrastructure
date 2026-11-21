//! End-to-end tests for `infra-harness-skillcache-driver` (`tools/skillcache_driver/`).
//! Exercises the driver binary over stdin/stdout with fake provider CLIs (no
//! live API calls) across all 6 arms, token normalization, workspace isolation,
//! shared observation, non-Git fixtures, and missing/failed preparations.
#![cfg(unix)]

use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const DRIVER: &str = env!("CARGO_BIN_EXE_infra-harness-skillcache-driver");

fn sandbox_usable() -> bool {
    cfg!(target_os = "linux")
        && Command::new("bwrap")
            .args([
                "--unshare-pid",
                "--unshare-net",
                "--ro-bind",
                "/",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "true",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

fn git(folder: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

struct Scratch(tempfile::TempDir);
impl Drop for Scratch {
    fn drop(&mut self) {
        infra_skillcache::package::release(self.0.path());
    }
}

struct HarnessStand {
    _dir: Scratch,
    _ws_dir: Scratch,
    _root: PathBuf,
    inputs: PathBuf,
    state_root: PathBuf,
    workspaces_root: PathBuf,
    verifiers_root: PathBuf,
    protocol_dir: PathBuf,
    protected_sentinel: PathBuf,
}

impl HarnessStand {
    fn new() -> Self {
        // Keep runtime inputs & state outside `/tmp` (bwrap `Profile::Discovery`
        // mounts `--tmpfs /tmp` after `runtime`/`readable`), while placing
        // workspaces in `/tmp` so non-Git `t2` sits outside this repo's `.git`.
        let dir = Scratch(tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap());
        let root = infra_core::fs::canonicalize(dir.0.path()).unwrap();
        let ws_dir = Scratch(tempfile::tempdir().unwrap());
        let ws_root = infra_core::fs::canonicalize(ws_dir.0.path()).unwrap();
        let inputs = root.join("inputs");
        let state_root = root.join("state");
        let workspaces_root = ws_root.join("workspaces");
        let verifiers_root = root.join("verifiers");
        let protocol_dir = root.join("protocol");
        for path in [
            &inputs,
            &state_root,
            &workspaces_root,
            &verifiers_root,
            &protocol_dir,
        ] {
            fs::create_dir_all(path).unwrap();
        }
        let protected_sentinel = verifiers_root.join("secret.txt");
        fs::write(&protected_sentinel, "VERIFIER_SECRET\n").unwrap();
        fs::write(protocol_dir.join("sealed.json"), "{}\n").unwrap();

        // Task 1: Git repository with `pdf` skill under both `.claude/skills` and `.agents/skills`.
        let task1 = inputs.join("fixtures/t1");
        fs::create_dir_all(task1.join(".claude/skills/pdf/scripts")).unwrap();
        fs::create_dir_all(task1.join(".agents/skills/pdf")).unwrap();
        fs::create_dir_all(task1.join("invoices")).unwrap();
        fs::write(task1.join("invoices/a.txt"), "Invoice 1\nTotal: 10.00\n").unwrap();
        fs::write(
            task1.join(".claude/skills/pdf/SKILL.md"),
            "---\nname: pdf\ndescription: Work with PDF invoices\n---\n# PDF\n\nExtract invoice rows.\n",
        )
        .unwrap();
        fs::write(
            task1.join(".claude/skills/pdf/scripts/extract.sh"),
            "mkdir -p out && printf 'rows\\n' > out/rows.txt\n",
        )
        .unwrap();
        fs::write(
            task1.join(".agents/skills/pdf/SKILL.md"),
            "---\nname: pdf\ndescription: Work with PDF invoices\n---\n# PDF\n\nExtract invoice rows.\n",
        )
        .unwrap();
        git(&task1, &["init", "-q"]);
        git(&task1, &["add", "-A"]);
        git(&task1, &["commit", "-q", "-m", "base"]);

        // Task 2: Non-Git folder with external skill overlay.
        let task2 = inputs.join("fixtures/t2");
        fs::create_dir_all(task2.join("invoices")).unwrap();
        fs::write(task2.join("invoices/b.txt"), "Invoice 2\nTotal: 20.00\n").unwrap();
        let skill_src = inputs.join("skills/pdf");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(
            skill_src.join("SKILL.md"),
            "---\nname: pdf\ndescription: Work with PDF invoices\n---\n# PDF\n\nExtract invoice rows.\n",
        )
        .unwrap();

        let stand = Self {
            _dir: dir,
            _ws_dir: ws_dir,
            _root: root,
            inputs,
            state_root,
            workspaces_root,
            verifiers_root,
            protocol_dir,
            protected_sentinel,
        };
        stand.write_fake_providers();
        stand.write_driver_config(&stand.state_root);
        stand
    }

    fn write_fake_providers(&self) {
        let bin = self.inputs.join("bin");
        fs::create_dir_all(&bin).unwrap();

        let hybrid_answer = json!({
            "form": "hybrid",
            "capsule": {
                "mu": {"name": "pdf-invoice-rows", "description": "Extract one row per invoice of a folder"},
                "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md"}]},
                "capabilities": {"network": "none", "runtimes": ["sh"]},
                "bindings": [{"name": "input_dir", "type": "path", "origin": "request", "description": "Folder with the invoices"}],
                "programs": [
                    {
                        "id": "extract",
                        "description": "Extract rows",
                        "entry": ["sh", "programs/extract/main.sh"],
                        "inputs": ["input_dir"],
                        "outputs": ["rows_file"],
                        "check": ["sh", "programs/extract/main.sh", "--check"],
                        "timeout_seconds": 30,
                        "read_only": false
                    },
                    {
                        "id": "verify",
                        "description": "Verify rows",
                        "entry": ["sh", "programs/verify/main.sh"],
                        "inputs": ["rows_file"],
                        "outputs": [],
                        "check": ["sh", "programs/verify/main.sh", "--check"],
                        "timeout_seconds": 30,
                        "read_only": true
                    }
                ],
                "steps": [{"kind": "program", "program": "extract"}],
                "verification": [{"kind": "program", "program": "verify"}]
            },
            "files": [
                {
                    "path": "programs/extract/main.sh",
                    "content": "if [ \"$1\" = \"--check\" ]; then printf '{\"status\":\"complete\",\"result\":{}}\\n'; exit 0; fi\nif [ ! -d \"$SKILLCACHE_WORKSPACE/invoices\" ]; then exit 2; fi\nmkdir -p \"$SKILLCACHE_WORKSPACE/out\"\nprintf 'rows\\n' > \"$SKILLCACHE_WORKSPACE/out/rows.txt\"\nprintf '{\"status\":\"complete\",\"result\":{\"rows_file\":\"out/rows.txt\"}}\\n'\n"
                },
                {
                    "path": "programs/verify/main.sh",
                    "content": "if [ \"$1\" = \"--check\" ]; then printf '{\"status\":\"complete\",\"result\":{}}\\n'; exit 0; fi\nif [ ! -f \"$SKILLCACHE_WORKSPACE/out/rows.txt\" ]; then exit 3; fi\nprintf '{\"status\":\"complete\",\"result\":{\"ok\":true}}\\n'\n"
                }
            ],
            "examples": {
                "bindings": {"input_dir": "invoices/"},
                "steps": {"0": {"result": {"rows_file": "out/rows.txt"}}}
            }
        });

        let textual_answer = json!({
            "form": "textual",
            "capsule": {
                "mu": {"name": "pdf-invoice-rows", "description": "Extract one row per invoice of a folder"},
                "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md"}]}
            },
            "markdown": "# Extract invoice rows\n\nWhen to use: a request for the rows of a folder of invoices.\n\n## Steps\n\n1. Read the invoice files in the folder.\n2. Write `out/rows.txt`.\n\n## Verify\n\n1. Confirm `out/rows.txt` exists.\n"
        });

        // Write canned compiler outputs into `inputs/bin` (mounted read-only in compile sandbox).
        let compiler_hybrid = [
            json!({
                "type": "assistant",
                "session_id": "comp-1",
                "message": {
                    "id": "msg_c1",
                    "role": "assistant",
                    "usage": {
                        "input_tokens": 500,
                        "output_tokens": 100,
                        "cache_read_input_tokens": 40,
                        "cache_creation": {
                            "ephemeral_5m_input_tokens": 30,
                            "ephemeral_1h_input_tokens": 20
                        }
                    },
                    "content": [{"type": "text", "text": "Compiled hybrid."}]
                }
            }),
            json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "result": "",
                "structured_output": hybrid_answer,
                "session_id": "comp-1",
                "usage": {
                    "input_tokens": 500,
                    "output_tokens": 100,
                    "cache_read_input_tokens": 40,
                    "cache_creation_input_tokens": 50
                },
                "total_cost_usd": 0.02,
                "num_turns": 1
            }),
        ]
        .map(|v| v.to_string())
        .join("\n");
        fs::write(
            bin.join("compiler-hybrid.jsonl"),
            format!("{compiler_hybrid}\n"),
        )
        .unwrap();

        let compiler_textual = [
            json!({
                "type": "assistant",
                "session_id": "comp-t1",
                "message": {
                    "id": "msg_ct1",
                    "role": "assistant",
                    "usage": {
                        "input_tokens": 300,
                        "output_tokens": 80,
                        "cache_read_input_tokens": 10,
                        "cache_creation": {
                            "ephemeral_5m_input_tokens": 15,
                            "ephemeral_1h_input_tokens": 5
                        }
                    },
                    "content": [{"type": "text", "text": "Compiled textual."}]
                }
            }),
            json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "result": "",
                "structured_output": textual_answer,
                "session_id": "comp-t1",
                "usage": {
                    "input_tokens": 300,
                    "output_tokens": 80,
                    "cache_read_input_tokens": 10,
                    "cache_creation_input_tokens": 20
                },
                "total_cost_usd": 0.01,
                "num_turns": 1
            }),
        ]
        .map(|v| v.to_string())
        .join("\n");
        fs::write(
            bin.join("compiler-textual.jsonl"),
            format!("{compiler_textual}\n"),
        )
        .unwrap();

        let claude_script = r##"#!/bin/sh
set -eu
dir=$(dirname "$0")
prompt=$(cat)

# Verify protected sentinel is unreadable when running inside sandbox.
if [ -n "${PILOT_PROTECTED_SENTINEL:-}" ] && [ -e "$PILOT_PROTECTED_SENTINEL" ]; then
  echo "PROTECTED_LEAK" >&2
  exit 99
fi

case " $* " in
  *" -p "*|*" --print "*) ;;
  *) exit 0 ;;
esac

case "$prompt" in
  *"You are the SkillCache compiler."*)
    if [ "${PILOT_FAIL_COMPILE:-}" = "1" ]; then
      echo "compiler forced fail" >&2
      exit 4
    fi
    case "$prompt" in
      *"textual.md"*) cat "$dir/compiler-textual.jsonl" ;;
      *) cat "$dir/compiler-hybrid.jsonl" ;;
    esac
    exit 0
    ;;
  *"skillcache-run run"*)
    printf '%s\n' "arm=executor" > .fake-arm.txt
    run_out=$(skillcache-run run '{"input_dir":"invoices"}')
    printf '%s\n' "$run_out" > .fake-run-out.json
    ;;
  *"run each program step with \`skillcache-tool\`"*)
    printf '%s\n' "arm=agent" > .fake-arm.txt
    tool_out=$(skillcache-tool extract --input '{"input_dir":"invoices"}')
    printf '%s\n' "$tool_out" > .fake-tool-out.json
    ;;
  *"This written procedure was compiled from the skill"*)
    printf '%s\n' "arm=written_procedure" > .fake-arm.txt
    mkdir -p out && printf 'rows\n' > out/rows.txt
    ;;
  *"Observed trace reference for this request (\`raw_trace\`"*)
    printf '%s\n' "arm=raw_trace" > .fake-arm.txt
    mkdir -p out && printf 'rows\n' > out/rows.txt
    ;;
  *)
    printf '%s\n' "arm=cold_or_observe" > .fake-arm.txt
    mkdir -p out && printf 'rows\n' > out/rows.txt
    printf '%s\n' "$prompt" > .fake-prompt.txt
    cat <<'EOF'
{"type":"system","subtype":"init","session_id":"nat-claude-1"}
{"type":"assistant","session_id":"nat-claude-1","message":{"id":"msg_1","role":"assistant","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":5,"ephemeral_1h_input_tokens":0}},"content":[{"type":"tool_use","id":"tu_1","name":"Read","input":{"file_path":".claude/skills/pdf/SKILL.md"}}]}}
{"type":"assistant","session_id":"nat-claude-1","message":{"id":"msg_1","role":"assistant","usage":{"input_tokens":100,"output_tokens":40,"cache_read_input_tokens":30,"cache_creation":{"ephemeral_5m_input_tokens":25,"ephemeral_1h_input_tokens":15}},"content":[{"type":"text","text":"reading skill"}]}}
{"type":"user","session_id":"nat-claude-1","message":{"content":[{"type":"tool_result","tool_use_id":"tu_1","content":"# PDF\nExtract invoice rows.","is_error":false}]}}
{"type":"assistant","session_id":"nat-claude-1","message":{"id":"msg_2","role":"assistant","usage":{"input_tokens":20,"output_tokens":10,"cache_read_input_tokens":5,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}},"content":[{"type":"tool_use","id":"tu_2","name":"Bash","input":{"command":"sh .claude/skills/pdf/scripts/extract.sh"}},{"type":"text","text":"Done."}]}}
{"type":"user","session_id":"nat-claude-1","message":{"content":[{"type":"tool_result","tool_use_id":"tu_2","content":"ok","is_error":false}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Done.","session_id":"nat-claude-1","total_cost_usd":0.01,"usage":{"input_tokens":120,"output_tokens":50,"cache_read_input_tokens":35,"cache_creation_input_tokens":40}}
EOF
    exit 0
    ;;
esac

# Warm / ablation assistant events (distinct message.ids with 5m/1h split)
cat <<'EOF'
{"type":"system","subtype":"init","session_id":"nat-claude-warm"}
{"type":"assistant","session_id":"nat-claude-warm","message":{"id":"msg_w1","role":"assistant","usage":{"input_tokens":40,"output_tokens":12,"cache_read_input_tokens":10,"cache_creation":{"ephemeral_5m_input_tokens":6,"ephemeral_1h_input_tokens":4}},"content":[{"type":"text","text":"Warm complete."}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Warm complete.","session_id":"nat-claude-warm","total_cost_usd":0.002,"usage":{"input_tokens":40,"output_tokens":12,"cache_read_input_tokens":10,"cache_creation_input_tokens":0}}
EOF
"##;
        let claude_path = bin.join("fake-claude");
        fs::write(&claude_path, claude_script).unwrap();
        fs::set_permissions(&claude_path, fs::Permissions::from_mode(0o755)).unwrap();

        let codex_script = r#"#!/bin/sh
set -eu
case " $* " in
  *" app-server "*)
    while IFS= read -r line; do
      case "$line" in
        *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"fake-codex"}}';;
        *'"method":"skills/list"'*)
          pwd_dir=$(pwd)
          printf '{"jsonrpc":"2.0","id":2,"result":{"data":[{"skills":[{"name":"pdf","description":"Work with PDF invoices","enabled":true,"scope":"repo","path":"%s/.agents/skills/pdf/SKILL.md"}]}]}}\n' "$pwd_dir"
          ;;
      esac
    done
    exit 0
    ;;
  *" exec "*) ;;
  *) exit 0 ;;
esac
prompt=$(cat)
printf '%s\n' "arm=codex" > .fake-arm.txt
printf '%s\n' "$prompt" > .fake-prompt.txt
mkdir -p out && printf 'rows\n' > out/rows.txt
cat <<'EOF'
{"type":"thread.started","thread_id":"codex-t1"}
{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Done."}}
{"type":"turn.completed","usage":{"input_tokens":150,"cached_input_tokens":40,"output_tokens":25}}
EOF
"#;
        let codex_path = bin.join("fake-codex");
        fs::write(&codex_path, codex_script).unwrap();
        fs::set_permissions(&codex_path, fs::Permissions::from_mode(0o755)).unwrap();

        let cursor_script = r#"#!/bin/sh
set -eu
case " $* " in
  *" -p "*|*" --print "*) ;;
  *) exit 0 ;;
esac
prompt=$(cat)
printf '%s\n' "arm=cursor" > .fake-arm.txt
mkdir -p out && printf 'rows\n' > out/rows.txt
cat <<'EOF'
{"type":"system","subtype":"init","session_id":"cursor-1"}
{"type":"assistant","session_id":"cursor-1","message":{"id":"cur_1","role":"assistant","usage":{"input_tokens":99,"output_tokens":33},"content":[{"type":"text","text":"Done."}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Done.","session_id":"cursor-1"}
EOF
"#;
        let cursor_path = bin.join("fake-cursor");
        fs::write(&cursor_path, cursor_script).unwrap();
        fs::set_permissions(&cursor_path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn write_driver_config(&self, state_root: &Path) {
        let cfg = json!({
            "schema": "skillcache-pilot-driver-v1",
            "state_root": state_root,
            "workspaces_root": self.workspaces_root,
            "engine": DRIVER,
            "session_timeout_seconds": 60,
            "models": {
                "claude-compiler": {
                    "provider": "claude",
                    "model": "claude-sonnet-4-6",
                    "permissions": "bypass",
                    "executable": "bin/fake-claude"
                },
                "claude-worker": {
                    "provider": "claude",
                    "model": "claude-haiku-4-5-20251001",
                    "permissions": "bypass",
                    "executable": "bin/fake-claude"
                },
                "codex-worker": {
                    "provider": "codex",
                    "model": "gpt-5.3-codex",
                    "permissions": "bypass",
                    "executable": "bin/fake-codex"
                },
                "cursor-worker": {
                    "provider": "cursor",
                    "permissions": "bypass",
                    "executable": "bin/fake-cursor"
                }
            },
            "tasks": {
                "t1": {"fixture": "fixtures/t1"},
                "t2": {"fixture": "fixtures/t2", "skills": ["skills/pdf"]}
            }
        });
        fs::write(
            self.inputs.join("driver.json"),
            serde_json::to_vec_pretty(&cfg).unwrap(),
        )
        .unwrap();
    }

    fn call_with_env(&self, request: &Value, envs: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(DRIVER);
        cmd.current_dir(&self._root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PILOT_PROTECTED_SENTINEL", &self.protected_sentinel);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn call(&self, request: &Value) -> Value {
        let output = self.call_with_env(request, &[]);
        assert!(
            output.status.success(),
            "driver failed ({:?}):\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
            panic!(
                "invalid driver stdout JSON ({err}): {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    fn protected(&self) -> Value {
        json!([self.verifiers_root, self.protocol_dir])
    }
}

#[test]
fn all_six_arms_and_shared_observation_work_end_to_end() {
    if !sandbox_usable() {
        eprintln!("skipped: this host does not allow bubblewrap namespaces");
        return;
    }
    let stand = HarnessStand::new();
    let seal = "a".repeat(64);
    let arms = [
        "cold",
        "raw_trace",
        "written_procedure",
        "skillcache_agent",
        "skillcache_executor",
        "skillcache_no_replay",
    ];

    // Prepare all 5 non-cold arms; verify shared observation & expected usages.
    let mut observe_usages = Vec::new();
    for arm in &arms {
        if *arm == "cold" {
            continue;
        }
        let compile_model = if *arm == "raw_trace" {
            Value::Null
        } else {
            json!("claude-compiler")
        };
        let prep = stand.call(&json!({
            "phase": "prepare",
            "seal_sha256": seal,
            "slot": format!("prep-{arm}"),
            "task": "t1",
            "skill": "pdf",
            "observe": "Extract the rows of invoices/",
            "arm": arm,
            "compile_model": compile_model,
            "executor_model": "claude-worker",
            "observe_model": "claude-worker",
            "inputs": stand.inputs,
            "protected": stand.protected(),
        }));
        assert_eq!(prep["prepared"], true, "{arm}: {prep}");
        assert!(prep["seconds"].as_f64().is_some());
        // Observation emitted msg_1 (100/40/30, 5m=25, 1h=15) + msg_2 (20/10/5, 0, 0).
        let obs = &prep["observe_usage"];
        assert_eq!(obs["input_tokens"], 120, "{arm}: {obs}");
        assert_eq!(obs["output_tokens"], 50, "{arm}: {obs}");
        assert_eq!(obs["cache_read_input_tokens"], 35, "{arm}: {obs}");
        assert_eq!(obs["cache_write_5m_input_tokens"], 25, "{arm}: {obs}");
        assert_eq!(obs["cache_write_1h_input_tokens"], 15, "{arm}: {obs}");
        assert_eq!(obs["cache_creation_input_tokens"], 40, "{arm}: {obs}");
        observe_usages.push(obs.clone());

        if *arm == "raw_trace" {
            assert_eq!(prep["usage"]["input_tokens"], 0);
        } else if *arm == "written_procedure" {
            assert_eq!(prep["usage"]["input_tokens"], 300);
            assert_eq!(prep["usage"]["cache_write_5m_input_tokens"], 15);
            assert_eq!(prep["usage"]["cache_write_1h_input_tokens"], 5);
            assert_eq!(prep["usage"]["cache_creation_input_tokens"], 20);
        } else {
            assert_eq!(prep["usage"]["input_tokens"], 500);
            assert_eq!(prep["usage"]["cache_write_5m_input_tokens"], 30);
            assert_eq!(prep["usage"]["cache_write_1h_input_tokens"], 20);
            assert_eq!(prep["usage"]["cache_creation_input_tokens"], 50);
        }
    }
    // Shared observation ran exactly once across all 5 arms.
    assert!(observe_usages.windows(2).all(|w| w[0] == w[1]));
    let observe_dirs = fs::read_dir(stand.state_root.join("observe"))
        .unwrap()
        .count();
    assert_eq!(observe_dirs, 1, "only one shared observation folder");

    // Verify `skillcache_no_replay` skipped Gate 3 while `skillcache_executor` ran Gate 3.
    let mut gate_counts = Vec::new();
    for entry in fs::read_dir(stand.state_root.join("prep")).unwrap() {
        let path = entry.unwrap().path();
        let meta: Value =
            serde_json::from_slice(&fs::read(path.join("prep.json")).unwrap()).unwrap();
        let arm = meta["arm"].as_str().unwrap().to_owned();
        if arm == "skillcache_no_replay" || arm == "skillcache_executor" {
            let db = rusqlite::Connection::open(path.join("harness/sessions.sqlite3")).unwrap();
            let gates: Vec<String> = {
                let mut stmt = db
                    .prepare("SELECT gate FROM skillcache_gates ORDER BY round,gate")
                    .unwrap();
                stmt.query_map([], |r| r.get(0))
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect()
            };
            gate_counts.push((arm, gates));
        }
    }
    let no_replay_gates = &gate_counts
        .iter()
        .find(|(a, _)| a == "skillcache_no_replay")
        .unwrap()
        .1;
    assert_eq!(no_replay_gates, &vec!["gate1", "gate2"]);
    let exec_gates = &gate_counts
        .iter()
        .find(|(a, _)| a == "skillcache_executor")
        .unwrap()
        .1;
    assert_eq!(exec_gates, &vec!["gate1", "gate2", "gate3"]);

    // Run attempts across all 6 arms; verify fresh workspaces, prompts, and step telemetry.
    let mut workspaces = std::collections::BTreeSet::new();
    for (idx, arm) in arms.iter().enumerate() {
        let compile_model = if *arm == "cold" || *arm == "raw_trace" {
            Value::Null
        } else {
            json!("claude-compiler")
        };
        let attempt = stand.call(&json!({
            "phase": "attempt",
            "seal_sha256": seal,
            "slot": format!("attempt-{idx}-{arm}"),
            "task": "t1",
            "skill": "pdf",
            "observe": "Extract the rows of invoices/",
            "probe": "exact",
            "request": "Extract the rows of invoices/",
            "arm": arm,
            "compile_model": compile_model,
            "executor_model": "claude-worker",
            "observe_model": "claude-worker",
            "position": idx,
            "inputs": stand.inputs,
            "protected": stand.protected(),
        }));
        assert_eq!(attempt["accepted"], true, "{arm}: {attempt}");
        // Every arm but `cold` says that its treatment ran; `cold` has none.
        assert_eq!(
            attempt["applied"],
            if *arm == "cold" {
                Value::Null
            } else {
                json!(true)
            },
            "{arm}: {attempt}"
        );
        let ws = PathBuf::from(attempt["workspace"].as_str().unwrap());
        assert!(ws.is_dir(), "{arm} workspace missing: {}", ws.display());
        assert!(workspaces.insert(ws.clone()), "reused workspace for {arm}");
        assert!(!ws.starts_with(&stand.state_root));
        assert!(!ws.starts_with(&stand.inputs));
        assert!(!ws.starts_with(&stand.verifiers_root));
        assert!(!ws.starts_with(&stand.protocol_dir));
        assert!(ws.join("out/rows.txt").is_file(), "{arm} produced output");

        let marker = fs::read_to_string(ws.join(".fake-arm.txt")).unwrap();
        match *arm {
            "cold" => {
                assert_eq!(marker.trim(), "arm=cold_or_observe");
                assert_eq!(attempt["usage"]["input_tokens"], 120);
                assert_eq!(attempt["model_calls"], 2);
                assert!(attempt["program_steps"].is_null());
                assert!(attempt["failed_steps"].is_null());
                let prompt = fs::read_to_string(ws.join(".fake-prompt.txt")).unwrap();
                assert!(prompt.contains("$pdf"), "{prompt}");
            }
            "raw_trace" => {
                assert_eq!(marker.trim(), "arm=raw_trace");
                assert_eq!(attempt["model_calls"], 1);
                assert!(attempt["program_steps"].is_null());
                assert!(attempt["failed_steps"].is_null());
            }
            "written_procedure" => {
                assert_eq!(marker.trim(), "arm=written_procedure");
                assert_eq!(attempt["model_calls"], 1);
                assert!(attempt["program_steps"].is_null());
                assert!(attempt["failed_steps"].is_null());
            }
            "skillcache_agent" => {
                assert_eq!(marker.trim(), "arm=agent");
                assert_eq!(attempt["model_calls"], 1);
                assert_eq!(attempt["program_steps"], 1);
                assert_eq!(attempt["failed_steps"], 0);
            }
            "skillcache_executor" | "skillcache_no_replay" => {
                assert_eq!(marker.trim(), "arm=executor");
                assert_eq!(attempt["model_calls"], 1);
                assert_eq!(attempt["program_steps"], 2);
                assert_eq!(attempt["failed_steps"], 0);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn codex_input_normalization_and_cursor_usage_nulls() {
    if !sandbox_usable() {
        eprintln!("skipped: this host does not allow bubblewrap namespaces");
        return;
    }
    let stand = HarnessStand::new();
    let seal = "b".repeat(64);

    let codex = stand.call(&json!({
        "phase": "attempt",
        "seal_sha256": seal,
        "slot": "codex-cold-1",
        "task": "t1",
        "skill": "pdf",
        "observe": "Extract rows",
        "probe": "exact",
        "request": "Extract rows",
        "arm": "cold",
        "compile_model": null,
        "executor_model": "codex-worker",
        "observe_model": "codex-worker",
        "position": 0,
        "inputs": stand.inputs,
        "protected": stand.protected(),
    }));
    assert_eq!(codex["accepted"], true, "{codex}");
    let usage = &codex["usage"];
    // 150 total input - 40 cached_input = 110 non-cached input; never emits cached_input_tokens.
    assert_eq!(usage["input_tokens"], 110, "{usage}");
    assert_eq!(usage["cache_read_input_tokens"], 40, "{usage}");
    assert_eq!(usage["output_tokens"], 25, "{usage}");
    assert!(usage["cache_write_5m_input_tokens"].is_null(), "{usage}");
    assert!(usage["cache_write_1h_input_tokens"].is_null(), "{usage}");
    assert!(usage["cache_creation_input_tokens"].is_null(), "{usage}");
    assert!(usage.get("cached_input_tokens").is_none(), "{usage}");
    assert!(codex["model_calls"].is_null(), "{codex}");

    let cursor = stand.call(&json!({
        "phase": "attempt",
        "seal_sha256": seal,
        "slot": "cursor-cold-1",
        "task": "t1",
        "skill": "pdf",
        "observe": "Extract rows",
        "probe": "exact",
        "request": "Extract rows",
        "arm": "cold",
        "compile_model": null,
        "executor_model": "cursor-worker",
        "observe_model": "cursor-worker",
        "position": 1,
        "inputs": stand.inputs,
        "protected": stand.protected(),
    }));
    assert_eq!(cursor["accepted"], true, "{cursor}");
    for key in [
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_write_5m_input_tokens",
        "cache_write_1h_input_tokens",
        "cache_creation_input_tokens",
    ] {
        assert!(
            cursor["usage"][key].is_null(),
            "{key} must be null: {cursor}"
        );
    }
    assert!(cursor["model_calls"].is_null(), "{cursor}");
}

#[test]
fn missing_or_failed_preparation_runs_cold_and_non_git_fixture_works() {
    if !sandbox_usable() {
        eprintln!("skipped: this host does not allow bubblewrap namespaces");
        return;
    }
    let stand = HarnessStand::new();
    let seal = "c".repeat(64);

    // Attempt with no prior `prepare` falls back cleanly to a cold run.
    let missing = stand.call(&json!({
        "phase": "attempt",
        "seal_sha256": seal,
        "slot": "missing-prep-1",
        "task": "t1",
        "skill": "pdf",
        "observe": "Extract rows",
        "probe": "exact",
        "request": "Extract rows",
        "arm": "skillcache_executor",
        "compile_model": "claude-compiler",
        "executor_model": "claude-worker",
        "observe_model": "claude-worker",
        "position": 0,
        "inputs": stand.inputs,
        "protected": stand.protected(),
    }));
    assert_eq!(missing["accepted"], true, "{missing}");
    // The arm ran without its capsule, and the answer says so.
    assert_eq!(missing["applied"], false, "{missing}");
    assert!(missing["program_steps"].is_null());
    assert!(missing["failed_steps"].is_null());
    let ws = PathBuf::from(missing["workspace"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(ws.join(".fake-arm.txt")).unwrap().trim(),
        "arm=cold_or_observe"
    );

    // Failed prepare (`prepared: false`) also runs subsequent attempts cold.
    let failed_prep_req = json!({
        "phase": "prepare",
        "seal_sha256": seal,
        "slot": "failed-prep",
        "task": "t2",
        "skill": "pdf",
        "observe": "Extract rows",
        "arm": "skillcache_executor",
        "compile_model": "claude-compiler",
        "executor_model": "claude-worker",
        "observe_model": "claude-worker",
        "inputs": stand.inputs,
        "protected": stand.protected(),
    });
    let out = stand.call_with_env(&failed_prep_req, &[("PILOT_FAIL_COMPILE", "1")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let failed_prep: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(failed_prep["prepared"], false, "{failed_prep}");

    let fallback = stand.call(&json!({
        "phase": "attempt",
        "seal_sha256": seal,
        "slot": "failed-prep-attempt",
        "task": "t2",
        "skill": "pdf",
        "observe": "Extract rows",
        "probe": "exact",
        "request": "Extract rows",
        "arm": "skillcache_executor",
        "compile_model": "claude-compiler",
        "executor_model": "claude-worker",
        "observe_model": "claude-worker",
        "position": 0,
        "inputs": stand.inputs,
        "protected": stand.protected(),
    }));
    assert_eq!(fallback["accepted"], true, "{fallback}");
    assert_eq!(fallback["applied"], false, "{fallback}");
    assert!(fallback["program_steps"].is_null());

    // Non-Git task `t2` prepares & executes warm when compilation succeeds.
    let seal_t2 = "d".repeat(64);
    let prep_t2 = stand.call(&json!({
        "phase": "prepare",
        "seal_sha256": seal_t2,
        "slot": "prep-t2",
        "task": "t2",
        "skill": "pdf",
        "observe": "Extract rows",
        "arm": "skillcache_executor",
        "compile_model": "claude-compiler",
        "executor_model": "claude-worker",
        "observe_model": "claude-worker",
        "inputs": stand.inputs,
        "protected": stand.protected(),
    }));
    assert_eq!(prep_t2["prepared"], true, "{prep_t2}");
    let warm_t2 = stand.call(&json!({
        "phase": "attempt",
        "seal_sha256": seal_t2,
        "slot": "attempt-t2",
        "task": "t2",
        "skill": "pdf",
        "observe": "Extract rows",
        "probe": "exact",
        "request": "Extract rows",
        "arm": "skillcache_executor",
        "compile_model": "claude-compiler",
        "executor_model": "claude-worker",
        "observe_model": "claude-worker",
        "position": 0,
        "inputs": stand.inputs,
        "protected": stand.protected(),
    }));
    assert_eq!(warm_t2["accepted"], true, "{warm_t2}");
    assert_eq!(warm_t2["program_steps"], 2);
    assert_eq!(warm_t2["failed_steps"], 0);

    // State root overlapping protected directory is refused with non-zero exit.
    stand.write_driver_config(&stand.verifiers_root.join("nested-state"));
    let bad = stand.call_with_env(
        &json!({
            "phase": "attempt",
            "seal_sha256": seal_t2,
            "slot": "overlap-1",
            "task": "t2",
            "skill": "pdf",
            "observe": "Extract rows",
            "probe": "exact",
            "request": "Extract rows",
            "arm": "cold",
            "compile_model": null,
            "executor_model": "claude-worker",
            "observe_model": "claude-worker",
            "position": 0,
            "inputs": stand.inputs,
            "protected": stand.protected(),
        }),
        &[],
    );
    assert!(!bad.status.success(), "overlapping state_root must fail");
}
