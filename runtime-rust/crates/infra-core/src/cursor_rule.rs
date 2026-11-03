//! Custody protocol for the registered generated Cursor rule.
//! It is independent of index/database construction and never mutates project policy.
use crate::{
    ErrorKind, Result, RuntimeContext, RuntimeError,
    bank::publication::{
        cursor::{CursorFiles, CursorImage},
        sha256,
    },
    domain::json,
};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const PROFILE: &str = "cursor-rule-custody-v1";
pub const RULE_LIMIT: usize = 16384;
pub const RECEIPT_LIMIT: usize = 4096;
const RULE: &str = ".cursor/rules/working-memory.mdc";
const RECEIPT: &str = "memory-bank/local/cursor-rule.json";
const HEADER: &str = "---\ndescription: Working memory - current-branch session context\nalwaysApply: true\n---\n\n# Working Memory (auto-rendered)\n\n";
const WARNING: &str =
    "Retrieved context is not authoritative - verify the source.\n\n\x60\x60\x60\n";
const END: &str = "\n\x60\x60\x60\n";
const EARLY: &[&str] = &[
    "---\ndescription: Working memory - Task Capsule as of end of previous turn\nalwaysApply: true\n---\n\n# Working Memory (auto-rendered)\n\n",
    "---\ndescription: Working memory - warming up, no Task Capsule rendered yet\nalwaysApply: true\n---\n\n# Working Memory (warming up)\n\n",
];

pub fn maintain(
    context: &RuntimeContext,
    action: &str,
    task_id: &str,
    phase: &str,
    approved_current_sha256: Option<&str>,
    input: &[u8],
) -> Result<Value> {
    maintain_input(
        context,
        action,
        task_id,
        phase,
        approved_current_sha256,
        false,
        input,
    )
}
pub fn retire(context: &RuntimeContext) -> Result<Value> {
    maintain_input(context, "retire", "", "prompt", None, false, &[])
}
pub fn maintain_report(
    context: &RuntimeContext,
    action: &str,
    task_id: &str,
    phase: &str,
    approved_current_sha256: Option<&str>,
    input: &[u8],
) -> Result<Value> {
    maintain_input(
        context,
        action,
        task_id,
        phase,
        approved_current_sha256,
        true,
        input,
    )
}
/// Hook input admission is separate from on-disk custody; malformed render input
/// becomes keep, so the current task can retain its prompt while foreign tasks retire.
pub fn maintain_input(
    context: &RuntimeContext,
    action: &str,
    task_id: &str,
    phase: &str,
    approved_current_sha256: Option<&str>,
    report_json: bool,
    input: &[u8],
) -> Result<Value> {
    let mut action = action.to_string();
    let mut capsule = String::new();
    if ["write", "migrate"].contains(&action.as_str()) {
        let limit = if report_json { 1024 * 1024 } else { RULE_LIMIT };
        if input.len() <= limit
            && let Ok(text) = std::str::from_utf8(input)
        {
            if !report_json {
                capsule = text.into();
            } else if let Ok(report) = read_render_report(text) {
                if report.withheld {
                    action = "keep".into();
                } else {
                    capsule = report.capsule;
                }
            }
        }
    }
    let mut result = object!({"profile":PROFILE,"state":"preserved","bytes_removed":0});
    if !["write", "keep", "retire", "prompt-held", "migrate"].contains(&action.as_str()) {
        return Ok(result);
    }
    if configuration_unavailable(context) {
        result["reason"] = "configuration_unavailable".into();
        return Ok(result);
    }
    if context.roots.attached() {
        result["state"] = "attached".into();
        return Ok(result);
    }
    if !cfg!(any(unix, windows)) {
        result["reason"] = "unsupported_custody".into();
        return Ok(result);
    }
    let operation = maintain_inner(
        context,
        &action,
        task_id,
        phase,
        &capsule,
        approved_current_sha256,
        &mut result,
    );
    if operation.is_err() && linked_rule_directory(&context.roots.state) {
        result["reason"] = "linked_directory".into();
    }
    // Errors contain no raw command, path, source excerpt or operating-system diagnostic.
    Ok(result)
}
fn set_state(result: &mut Value, state: &str) {
    result["state"] = state.into();
}
fn set_reason(result: &mut Value, reason: &str) {
    result["reason"] = reason.into();
}
fn valid_task(task: &str) -> bool {
    regex::Regex::new(r"^[\p{L}\p{N}][\p{L}\p{N}_\u{0300}-\u{036f}./#+-]{0,199}$")
        .is_ok_and(|expression| expression.is_match(task))
}
fn hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn edited_generation(raw: &[u8]) -> bool {
    raw.starts_with(HEADER.as_bytes())
        || EARLY
            .iter()
            .any(|header| raw.starts_with(header.as_bytes()))
}
fn machine_envelope(raw: &[u8]) -> Option<(String, String)> {
    let text = std::str::from_utf8(raw).ok()?;
    if text.contains('\0') {
        return None;
    }
    let rest = text.strip_prefix(HEADER)?;
    let (header, tail) = rest.split_once('\n')?;
    let expression=regex::Regex::new(r"^(Session context retrieved for a recent prompt|Session context as of end of previous turn) \(task: ([A-Za-z0-9][A-Za-z0-9._/-]{0,199})\)\.$").ok()?;
    let captures = expression.captures(header)?;
    let body = tail.strip_prefix(WARNING)?.strip_suffix(END)?;
    if !body.starts_with("working:") || body.contains(END) || body.ends_with("\n\x60\x60\x60") {
        return None;
    }
    Some((
        captures.get(2)?.as_str().into(),
        if captures
            .get(1)?
            .as_str()
            .starts_with("Session context retrieved")
        {
            "prompt"
        } else {
            "turn"
        }
        .into(),
    ))
}
fn receipt(raw: &[u8]) -> Result<Value> {
    let compatible = receipt_surrogates(raw);
    let value = crate::results::strict_json(&compatible)?;
    let fields = [
        "profile",
        "root",
        "root_identity",
        "rules_identity",
        "file_identity",
        "sha256",
        "task",
        "phase",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let map = value
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Unsupported Cursor receipt"))?;
    if map.keys().map(String::as_str).collect::<BTreeSet<_>>() != fields
        || value["profile"] != PROFILE
        || !value["root"].is_string()
        || !value["task"].as_str().is_some_and(valid_task)
        || !value["phase"]
            .as_str()
            .is_some_and(|phase| ["prompt", "turn", "session"].contains(&phase))
        || !value["sha256"].as_str().is_some_and(hex_digest)
    {
        return Err(RuntimeError::invalid("Unsupported Cursor receipt"));
    }
    for (name, width) in [
        ("root_identity", 2),
        ("rules_identity", 2),
        ("file_identity", 5),
    ] {
        let image = value[name]
            .as_array()
            .filter(|image| image.len() == width)
            .ok_or_else(|| RuntimeError::invalid("Unsupported Cursor receipt identity"))?;
        if image.iter().any(|item| {
            !item.as_number().is_some_and(|number| {
                let token = number.to_string();
                !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit())
            })
        }) {
            return Err(RuntimeError::invalid("Unsupported Cursor receipt identity"));
        }
    }
    Ok(value)
}
fn receipt_bytes(note: &Value) -> Result<Vec<u8>> {
    Ok(json::python_default(note, false, true, true)?.into_bytes())
}
fn root_text(root: &Path) -> Result<String> {
    root.to_str()
        .map(String::from)
        .ok_or_else(|| RuntimeError::invalid("Cursor receipt root is not Unicode"))
}
fn fresh_receipt(
    root: &Path,
    files: &mut CursorFiles,
    current: &CursorImage,
    task: &str,
    phase: &str,
) -> Result<Value> {
    let rules = files
        .directory_identity(".cursor/rules", false)?
        .ok_or_else(|| RuntimeError::conflict("Cursor rules directory absent"))?;
    Ok(
        object!({"profile":PROFILE,"root":root_text(root)?,"root_identity":files.root_identity(),"rules_identity":rules,
        "file_identity":current.receipt_identity,"sha256":sha256(&current.bytes),"task":task,"phase":phase}),
    )
}
fn initial_image(
    root: &Path,
    files: &mut CursorFiles,
    relative: &str,
    limit: usize,
    result: &mut Value,
) -> Result<Option<CursorImage>> {
    match files.image(relative, limit, 1) {
        Ok(image) => Ok(image),
        Err(original) => {
            if !cfg!(unix) {
                return Err(original);
            }
            // A generic hardlink is never adopted; only a complete registered
            // producer envelope or validated receipt can enter this recovery.
            if !registered_slot(root, files)? {
                return Err(original);
            }
            let _lock = files.lock()?;
            let Some(current) = files.image(relative, limit, 2)? else {
                return Err(original);
            };
            if relative == RULE {
                if machine_envelope(&current.bytes).is_none() {
                    return Err(original);
                }
            } else {
                receipt(&current.bytes)?;
            }
            let Some(alias) = files.legacy_alias(relative, &current)? else {
                return Err(original);
            };
            if !git_untracked(root, &[alias.as_str()])? {
                return Err(original);
            }
            files.check()?;
            set_state(result, "incomplete");
            if !files.reconcile_alias(relative, &alias, &current, limit)? {
                return Err(RuntimeError::conflict("Unsupported Cursor alias custody"));
            }
            files.image(relative, limit, 1)
        }
    }
}
fn maintain_inner(
    context: &RuntimeContext,
    requested_action: &str,
    task: &str,
    phase: &str,
    capsule: &str,
    approved: Option<&str>,
    result: &mut Value,
) -> Result<()> {
    let mut files = CursorFiles::new(&context.roots.state)?;
    let root = std::fs::canonicalize(&context.roots.state)?;
    if root != std::fs::canonicalize(&context.roots.tooling)?
        || root != std::fs::canonicalize(&context.roots.project)?
        || linked_state_entry(&root)
    {
        return Ok(());
    }
    // Pin pre-existing chains even when the current leaves are absent.
    for directory in [
        ".cursor",
        ".cursor/rules",
        "memory-bank",
        "memory-bank/local",
    ] {
        files.directory_identity(directory, false)?;
    }
    let current = initial_image(&root, &mut files, RULE, RULE_LIMIT, result)?;
    let proof = initial_image(&root, &mut files, RECEIPT, RECEIPT_LIMIT, result)?;
    let mut action = requested_action.to_string();
    let approved_migration = action == "migrate";
    if approved_migration {
        if !current.as_ref().is_some_and(|image| {
            approved
                .is_some_and(|approved| hex_digest(approved) && sha256(&image.bytes) == approved)
        }) {
            set_reason(result, "migration_unapproved");
            return Ok(());
        }
        action = "write".into();
    }
    if current.is_none() && proof.is_none() && action != "write" {
        set_state(result, "absent");
        return Ok(());
    }
    let mut note = if !approved_migration {
        proof.as_ref().and_then(|proof| receipt(&proof.bytes).ok())
    } else {
        None
    };
    let matches = note.as_ref().is_some_and(|note| {
        current
            .as_ref()
            .is_none_or(|image| note["sha256"].as_str() == Some(sha256(&image.bytes).as_str()))
    });
    let mut recovered = false;
    let mut edited = false;
    if current.is_some() && !matches && !approved_migration {
        let image = current
            .as_ref()
            .ok_or_else(|| RuntimeError::unavailable("Cursor image absent"))?;
        let machine = machine_envelope(&image.bytes).or_else(|| {
            if edited_generation(&image.bytes) {
                edited = true;
                Some((String::new(), String::new()))
            } else {
                None
            }
        });
        if machine.is_none() || !registered_slot(&root, &mut files)? {
            set_reason(
                result,
                if proof.is_some() {
                    "custody_changed"
                } else {
                    "ownership_unverified"
                },
            );
            return Ok(());
        }
        let (machine_task, machine_phase) =
            machine.ok_or_else(|| RuntimeError::invalid("Cursor producer envelope unavailable"))?;
        note = Some(fresh_receipt(
            &root,
            &mut files,
            image,
            &machine_task,
            &machine_phase,
        )?);
        recovered = true;
    }
    if current.is_none()
        && proof.is_some()
        && note.is_none()
        && !approved_migration
        && action != "write"
    {
        set_reason(result, "receipt_unavailable");
        return Ok(());
    }
    let relocated = if let Some(note) = &note {
        recovered
            || note["root"].as_str() != Some(root_text(&root)?.as_str())
            || note["root_identity"] != object!(files.root_identity())
            || files
                .directory_identity(".cursor/rules", false)?
                .is_none_or(|identity| note["rules_identity"] != object!(identity))
            || current
                .as_ref()
                .is_some_and(|image| note["file_identity"] != image.receipt_identity)
    } else {
        false
    };
    let valid = valid_task(task);
    let mut reconcile_state = None::<String>;
    if action == "prompt-held" {
        let state = if current.is_some()
            && note
                .as_ref()
                .is_some_and(|note| valid && note["task"] == task && note["phase"] == "prompt")
        {
            "kept"
        } else if current.is_none() {
            "absent"
        } else {
            "preserved"
        };
        if !relocated || state != "kept" {
            set_state(result, state);
            return Ok(());
        }
        action = "reconcile".into();
        reconcile_state = Some(state.into());
    }
    if action == "keep" || action == "write" && !valid {
        let Some(note) = &note else { return Ok(()) };
        if current.is_some() && valid && note["task"] == task {
            if !relocated {
                set_state(result, "kept");
                return Ok(());
            }
            action = "reconcile".into();
            reconcile_state = Some("kept".into());
        } else {
            action = "retire".into();
        }
    }
    let mut payload = vec![];
    if action == "write" {
        if !["prompt", "turn", "session"].contains(&phase) {
            return Ok(());
        }
        if !capsule.starts_with("working:") || capsule.contains('\0') {
            *result = maintain(context, "keep", task, "prompt", None, &[])?;
            return Ok(());
        }
        if current.is_some()
            && phase == "turn"
            && note
                .as_ref()
                .is_some_and(|note| note["task"] == task && note["phase"] == "prompt")
        {
            if !relocated {
                set_state(result, "kept");
                return Ok(());
            }
            action = "reconcile".into();
            reconcile_state = Some("kept".into());
        }
        let marker = if phase == "prompt" {
            "Session context retrieved for a recent prompt"
        } else {
            "Session context as of end of previous turn"
        };
        let mut body = capsule;
        while let Some(previous) = body.strip_suffix('\n') {
            body = previous;
        }
        payload = format!("{HEADER}{marker} (task: {task}).\n{WARNING}{body}{END}").into_bytes();
        if payload.len() > RULE_LIMIT {
            *result = maintain(context, "keep", task, "prompt", None, &[])?;
            return Ok(());
        }
        if current.as_ref().is_some_and(|image| image.bytes == payload)
            && note.as_ref().is_some_and(|note| {
                relocated
                    || current
                        .as_ref()
                        .is_some_and(|image| note["file_identity"] == image.receipt_identity)
            })
        {
            if !relocated {
                set_state(result, "unchanged");
                return Ok(());
            }
            if reconcile_state.is_none() {
                action = "reconcile".into();
                reconcile_state = Some("unchanged".into());
            }
        }
    }
    if !git_untracked(&root, &[])? {
        return Ok(());
    }
    files.check()?;
    let _lock = files.lock()?;
    files.check()?;
    if !git_untracked(&root, &[])?
        || files.image(RULE, RULE_LIMIT, 1)? != current
        || files.image(RECEIPT, RECEIPT_LIMIT, 1)? != proof
    {
        return Ok(());
    }
    files.check()?;
    if action == "reconcile" {
        set_state(result, "incomplete");
        let current = current
            .as_ref()
            .ok_or_else(|| RuntimeError::conflict("Cursor reconciliation leaf absent"))?;
        files.directory_identity("memory-bank/local", true)?;
        let mut note =
            note.ok_or_else(|| RuntimeError::invalid("Cursor reconciliation receipt absent"))?;
        note["root"] = root_text(&root)?.into();
        note["root_identity"] = object!(files.root_identity());
        note["rules_identity"] = object!(
            files
                .directory_identity(".cursor/rules", false)?
                .ok_or_else(|| RuntimeError::conflict("Cursor rules disappeared"))?
        );
        note["file_identity"] = current.receipt_identity.clone();
        let receipt = receipt_bytes(&note)?;
        files.check()?;
        if files.image(RULE, RULE_LIMIT, 1)?.as_ref() != Some(current) {
            return Ok(());
        }
        files.publish(RECEIPT, &receipt, proof.as_ref(), RECEIPT_LIMIT)?;
        let published = files.image(RECEIPT, RECEIPT_LIMIT, 1)?;
        files.check()?;
        if files.image(RULE, RULE_LIMIT, 1)?.as_ref() != Some(current)
            || published
                .as_ref()
                .is_none_or(|image| image.bytes != receipt)
        {
            return Ok(());
        }
        set_state(result, reconcile_state.as_deref().unwrap_or("kept"));
        return Ok(());
    }
    if action == "retire" {
        if let Some(current) = &current {
            if edited {
                result["preserved_edit"] = files.preserve_edit(&current.bytes)?.into();
            }
            files.unlink(RULE, current, RULE_LIMIT)?;
            set_state(result, "incomplete");
            result["bytes_removed"] = object!(current.bytes.len());
        }
        if let Some(proof) = &proof {
            match files.unlink(RECEIPT, proof, RECEIPT_LIMIT) {
                Ok(()) => {}
                Err(error) if error.kind == ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        set_state(
            result,
            if current.is_some() {
                "removed"
            } else {
                "absent"
            },
        );
        return Ok(());
    }
    for directory in [".cursor/rules", "memory-bank/local"] {
        files.directory_identity(directory, true)?;
    }
    files.check()?;
    files.sweep_temporaries(".cursor/rules")?;
    files.sweep_temporaries("memory-bank/local")?;
    set_state(result, "incomplete");
    let same_body = current.as_ref().is_some_and(|image| image.bytes == payload);
    if !same_body {
        if edited && let Some(current) = &current {
            result["preserved_edit"] = files.preserve_edit(&current.bytes)?.into();
        }
        files.publish(RULE, &payload, current.as_ref(), RULE_LIMIT)?;
    }
    let published = files.image(RULE, RULE_LIMIT, 1)?;
    files.check()?;
    if published
        .as_ref()
        .is_none_or(|image| image.bytes != payload)
        || same_body && published != current
    {
        return Ok(());
    }
    let published =
        published.ok_or_else(|| RuntimeError::unavailable("Cursor publication absent"))?;
    let note = fresh_receipt(&root, &mut files, &published, task, phase)?;
    files.publish(
        RECEIPT,
        &receipt_bytes(&note)?,
        proof.as_ref(),
        RECEIPT_LIMIT,
    )?;
    set_state(result, if same_body { "unchanged" } else { "written" });
    Ok(())
}
fn linked_state_entry(root: &Path) -> bool {
    std::iter::once(root.to_path_buf())
        .chain(
            [
                "project-brain",
                "memory-bank",
                "launch",
                "accelerator-attach.json",
            ]
            .map(|name| root.join(name)),
        )
        .any(|path| std::fs::symlink_metadata(path).is_ok_and(|image| crate::fs::linked(&image)))
}
fn linked_rule_directory(root: &Path) -> bool {
    [root.join(".cursor"), root.join(".cursor/rules")]
        .iter()
        .any(|path| std::fs::symlink_metadata(path).is_ok_and(|image| crate::fs::linked(&image)))
}
fn configured(name: &str) -> Option<PathBuf> {
    let raw = std::env::var_os(name)?;
    let text = raw.to_string_lossy();
    let text = text.trim_matches(crate::results::py_space);
    if text.is_empty() {
        return None;
    }
    Some(expand_user(text))
}
fn expand_user(value: &str) -> PathBuf {
    if value == "~" || value.starts_with("~/") || value.starts_with("~\\") && cfg!(windows) {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
        if let Some(home) = home {
            return if value.len() == 1 {
                PathBuf::from(home)
            } else {
                PathBuf::from(home).join(&value[2..])
            };
        }
    }
    #[cfg(unix)]
    if let Some(stripped) = value.strip_prefix('~') {
        use std::os::unix::ffi::OsStringExt;
        let (user, tail) = stripped
            .split_once('/')
            .map(|(user, tail)| (user, Some(tail)))
            .unwrap_or((stripped, None));
        let selected = if user.is_empty() {
            None
        } else {
            std::ffi::CString::new(user).ok()
        };
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut out = std::ptr::null_mut();
        let mut buffer = vec![0u8; 65536];
        let code = unsafe {
            match selected {
                Some(ref user) => libc::getpwnam_r(
                    user.as_ptr(),
                    pwd.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut out,
                ),
                None => libc::getpwuid_r(
                    libc::getuid(),
                    pwd.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut out,
                ),
            }
        };
        if code == 0 && !out.is_null() {
            let pwd = unsafe { pwd.assume_init() };
            if !pwd.pw_dir.is_null() {
                let home = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }
                    .to_bytes()
                    .to_vec();
                let home = PathBuf::from(OsString::from_vec(home));
                return tail.map(|tail| home.join(tail)).unwrap_or(home);
            }
        }
    }
    PathBuf::from(value)
}
fn configuration_unavailable(context: &RuntimeContext) -> bool {
    let Some(home) = configured("ACCELERATOR_HOME") else {
        return false;
    };
    match (
        crate::fs::canonical_or_missing(&home),
        crate::fs::canonical_or_missing(&context.roots.tooling),
    ) {
        (Ok(home), Ok(tooling)) if home == tooling => {}
        _ => return false,
    }
    let (Some(state), Some(project)) = (
        configured("ACCELERATOR_STATE_DIR"),
        configured("ACCELERATOR_PROJECT_DIR"),
    ) else {
        return true;
    };
    if !state.is_absolute()
        || !project.is_absolute()
        || !project.is_dir()
        || linked_state_entry(&state)
    {
        return true;
    }
    match (
        crate::fs::canonical_or_missing(&state),
        std::fs::canonicalize(&project),
    ) {
        (Ok(state), Ok(project)) => state == project || state.starts_with(project),
        _ => true,
    }
}
fn registered_slot(root: &Path, files: &mut CursorFiles) -> Result<bool> {
    let listed = match files.image(".gitignore", 65536, 1)? {
        Some(image) => std::str::from_utf8(&image.bytes)
            .map_err(|_| RuntimeError::invalid("Cursor registration is not UTF-8"))?
            .split(|c| {
                matches!(
                    c,
                    '\n' | '\r'
                        | '\u{b}'
                        | '\u{c}'
                        | '\u{1c}'
                        | '\u{1d}'
                        | '\u{1e}'
                        | '\u{85}'
                        | '\u{2028}'
                        | '\u{2029}'
                )
            })
            .any(|line| {
                [
                    ".cursor/rules/working-memory.mdc",
                    "/.cursor/rules/working-memory.mdc",
                ]
                .contains(&line.trim_matches(crate::results::py_space))
            }),
        None => false,
    };
    if !listed && !source_git_ignored(root)? {
        return Ok(false);
    }
    git_untracked(root, &[])
}

struct GitOutput {
    code: i32,
    stdout: Vec<u8>,
}
fn reap_after_kill(mut child: Child) {
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                thread::spawn(move || {
                    let _ = child.wait();
                });
                return;
            }
        }
    }
}
/// The only executable is Git, with fixed subcommands and argument-vector input.
/// Protected policy is read without command/environment injection; the private
/// custody query disables repository-external config until that policy admits one root.
fn git_read(
    root: &Path,
    arguments: &[OsString],
    private_config: bool,
    input: Option<&[u8]>,
    limit: Option<usize>,
    timeout: Duration,
) -> Result<GitOutput> {
    if timeout.is_zero() {
        return Err(RuntimeError::unavailable("Cursor Git deadline elapsed"));
    }
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut command = Command::new("git");
    command
        .args(["--no-optional-locks", "--no-pager"])
        .args(["-C"])
        .arg(root)
        .args(arguments);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    if private_config {
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", null);
    }
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .stdout(if limit.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command
        .spawn()
        .map_err(|_| RuntimeError::unavailable("Cursor Git query unavailable"))?;
    if let Some(input) = input {
        let write = child
            .stdin
            .take()
            .ok_or_else(|| RuntimeError::unavailable("Cursor Git input unavailable"))
            .and_then(|mut stdin| stdin.write_all(input).map_err(Into::into));
        if let Err(error) = write {
            reap_after_kill(child);
            return Err(error);
        }
    }
    let receiver = if let Some(limit) = limit {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RuntimeError::unavailable("Cursor Git output unavailable"))?;
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let read = stdout.take(limit as u64 + 1).read_to_end(&mut bytes);
            let _ = tx.send(read.map(|_| bytes));
        });
        Some(rx)
    } else {
        None
    };
    let deadline = Instant::now() + timeout;
    let bytes = if let Some(receiver) = receiver {
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(bytes)) if limit.is_some_and(|limit| bytes.len() <= limit) => bytes,
            _ => {
                reap_after_kill(child);
                return Err(RuntimeError::unavailable("Cursor Git output unavailable"));
            }
        }
    } else {
        vec![]
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(GitOutput {
                    code: status.code().unwrap_or(-1),
                    stdout: bytes,
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                reap_after_kill(child);
                return Err(RuntimeError::unavailable("Cursor Git query unavailable"));
            }
        }
    }
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn normcase(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| RuntimeError::invalid("Cursor Git policy path is not Unicode"))?;
    Ok(if cfg!(windows) {
        text.replace('/', "\\").to_lowercase()
    } else {
        text.into()
    })
}
fn protected_safe_root(root: &Path) -> Result<bool> {
    let output = git_read(
        root,
        &args(&[
            "config",
            "--show-scope",
            "--includes",
            "--null",
            "--get-all",
            "safe.directory",
        ]),
        false,
        None,
        Some(65536),
        Duration::from_secs(1),
    )?;
    if ![0, 1].contains(&output.code)
        || output.stdout.iter().filter(|byte| **byte == 0).count() > 512
        || !output.stdout.is_empty() && output.stdout.last() != Some(&0)
    {
        return Ok(false);
    }
    let target = normcase(&std::fs::canonicalize(root)?)?;
    let mut allowed = false;
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| RuntimeError::invalid("Cursor Git policy is not UTF-8"))?;
    let fields = if text.is_empty() {
        vec![]
    } else {
        text[..text.len() - 1].split('\0').collect::<Vec<_>>()
    };
    if fields.len() % 2 != 0 {
        return Ok(false);
    }
    for pair in fields.as_chunks::<2>().0 {
        let (scope, value) = (pair[0], pair[1]);
        if !["system", "global"].contains(&scope) {
            continue;
        }
        if value.is_empty() {
            allowed = false;
            continue;
        }
        if value == "*" {
            allowed = true;
            continue;
        }
        if value.contains(['\r', '\n', '\0']) || value.starts_with("%(prefix)") {
            continue;
        }
        let subtree = value.ends_with("/*");
        let candidate = if subtree {
            &value[..value.len() - 2]
        } else {
            value
        };
        let path = if candidate.starts_with("~/") {
            expand_user(candidate)
        } else {
            PathBuf::from(candidate)
        };
        if !path.is_absolute() {
            continue;
        }
        let named = normcase(&crate::fs::canonical_or_missing(&path)?)?;
        let separator = std::path::MAIN_SEPARATOR;
        let mut base = named.as_str();
        while let Some(previous) = base.strip_suffix(separator) {
            base = previous;
        }
        if named == target || subtree && target.starts_with(&format!("{base}{separator}")) {
            allowed = true;
        }
    }
    Ok(allowed)
}
fn git_untracked(root: &Path, extra: &[&str]) -> Result<bool> {
    const SELECTORS: &[&str] = &[
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_SHALLOW_FILE",
        "GIT_NAMESPACE",
    ];
    if SELECTORS
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return Ok(false);
    }
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut trust = None::<String>;
    for name in [RULE, RECEIPT].into_iter().chain(extra.iter().copied()) {
        let make = |trust: Option<&str>| {
            let mut command = args(&["--literal-pathspecs"]);
            if let Some(trust) = trust {
                command.extend(args(&["-c", trust]));
            }
            command.extend(args(&[
                "-c",
                &format!("core.hooksPath={null}"),
                "-c",
                "core.fsmonitor=false",
                "ls-files",
                "--error-unmatch",
                "--",
                name,
            ]));
            command
        };
        let mut output = git_read(
            root,
            &make(trust.as_deref()),
            true,
            None,
            None,
            Duration::from_secs(1),
        )?;
        if output.code == 128 && trust.is_none() && protected_safe_root(root)? {
            trust = Some(format!(
                "safe.directory={}",
                root_text(&std::fs::canonicalize(root)?)?
            ));
            output = git_read(
                root,
                &make(trust.as_deref()),
                true,
                None,
                None,
                Duration::from_secs(1),
            )?;
        }
        if output.code != 1 {
            return Ok(false);
        }
    }
    Ok(true)
}
#[derive(PartialEq, Eq)]
struct SourceFrame {
    top: PathBuf,
    admin: PathBuf,
    prefix: String,
}
fn source_frame(root: &Path, deadline: Instant) -> Result<Option<SourceFrame>> {
    let root = std::fs::canonicalize(root)?;
    let mut checkout = None;
    for directory in root.ancestors() {
        let marker = directory.join(".git");
        let image = match std::fs::symlink_metadata(&marker) {
            Ok(image) => image,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let image = if crate::fs::linked(&image) {
            std::fs::metadata(&marker)?
        } else {
            image
        };
        if image.is_dir() {
            let head = std::fs::symlink_metadata(marker.join("HEAD"));
            if !head.is_ok_and(|head| head.is_file() || head.file_type().is_symlink())
                || !marker.join("objects").is_dir()
                || !marker.join("refs").is_dir()
            {
                continue;
            }
        } else if !image.is_file() {
            return Err(RuntimeError::unavailable(
                "Cursor source Git marker unavailable",
            ));
        }
        checkout = Some(directory.to_path_buf());
        break;
    }
    let Some(checkout) = checkout else {
        return Ok(None);
    };
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let arguments = args(&[
        "-c",
        &format!("core.hooksPath={null}"),
        "-c",
        "core.fsmonitor=false",
        "rev-parse",
        "--show-toplevel",
        "--show-prefix",
        "--absolute-git-dir",
    ]);
    let output = git_read(
        &root,
        &arguments,
        false,
        None,
        Some(16384),
        deadline.saturating_duration_since(Instant::now()),
    )?;
    if output.code != 0 {
        return Err(RuntimeError::unavailable(
            "Cursor source Git frame unavailable",
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| RuntimeError::invalid("Cursor source Git frame is not Unicode"))?;
    let lines = text.split('\n').collect::<Vec<_>>();
    if lines.len() != 4 || !lines[3].is_empty() {
        return Err(RuntimeError::unavailable(
            "Cursor source Git frame incomplete",
        ));
    }
    let top = PathBuf::from(lines[0]);
    let admin = PathBuf::from(lines[2]);
    if !top.is_absolute() || !admin.is_absolute() {
        return Err(RuntimeError::unavailable(
            "Cursor source Git frame is not absolute",
        ));
    }
    let top = std::fs::canonicalize(top)?;
    let admin = std::fs::canonicalize(admin)?;
    let relative = root
        .strip_prefix(&top)
        .map_err(|_| RuntimeError::unavailable("Cursor source Git frame escapes root"))?;
    let mut prefix = relative
        .to_str()
        .ok_or_else(|| RuntimeError::invalid("Cursor Git prefix is not Unicode"))?
        .replace('\\', "/");
    if !prefix.is_empty() {
        prefix.push('/');
    }
    if top != checkout || prefix != lines[1] || !admin.is_dir() {
        return Err(RuntimeError::unavailable(
            "Cursor source Git frame disagrees with project",
        ));
    }
    Ok(Some(SourceFrame { top, admin, prefix }))
}
fn source_git_ignored(root: &Path) -> Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let Some(before) = source_frame(root, deadline)? else {
        return Ok(false);
    };
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut arguments = args(&[
        "-c",
        &format!("core.hooksPath={null}"),
        "-c",
        "core.fsmonitor=false",
    ]);
    let mut gitdir = OsString::from("--git-dir=");
    gitdir.push(&before.admin);
    let mut worktree = OsString::from("--work-tree=");
    worktree.push(&before.top);
    arguments.extend([gitdir, worktree]);
    arguments.extend(args(&["check-ignore", "--stdin", "-z"]));
    let mut payload = RULE.as_bytes().to_vec();
    payload.push(0);
    let output = git_read(
        root,
        &arguments,
        false,
        Some(&payload),
        Some(payload.len()),
        deadline.saturating_duration_since(Instant::now()),
    )?;
    if source_frame(root, deadline)?.as_ref() != Some(&before) {
        return Err(RuntimeError::conflict("Cursor source Git frame changed"));
    }
    if output.code == 1 && output.stdout.is_empty() {
        return Ok(false);
    }
    if output.code != 0 || output.stdout != payload {
        return Err(RuntimeError::unavailable(
            "Cursor source Git policy output invalid",
        ));
    }
    Ok(true)
}

/// JSON report admission follows the Python document boundary: duplicate members
/// use the last value, nonfinite constants are legal, and escaped unpaired
/// surrogates in irrelevant members do not invalidate a valid capsule. Only the
/// selected delivery strings are decoded into Rust strings; invalid capsule
/// Unicode therefore takes the same keep path as strict UTF-8 publication.
struct RenderReport {
    capsule: String,
    withheld: bool,
}
enum RenderNode {
    Other,
    Text(String),
    Object(std::collections::BTreeMap<String, RenderNode>),
}
impl RenderNode {
    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(map) => map.get(key),
            _ => None,
        }
    }
    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }
}
#[derive(Clone, Copy)]
enum Wanted {
    Root,
    Capsule,
    Gate,
    Text,
    Ignore,
}
fn read_render_report(text: &str) -> Result<RenderReport> {
    struct Parser<'a> {
        bytes: &'a [u8],
        at: usize,
    }
    impl Parser<'_> {
        fn space(&mut self) {
            while self
                .bytes
                .get(self.at)
                .is_some_and(|byte| b" \r\n\t".contains(byte))
            {
                self.at += 1;
            }
        }
        fn take(&mut self, byte: u8) -> bool {
            if self.bytes.get(self.at) == Some(&byte) {
                self.at += 1;
                true
            } else {
                false
            }
        }
        fn string(&mut self) -> Result<&[u8]> {
            let start = self.at;
            if !self.take(b'"') {
                return Err(RuntimeError::invalid("Invalid Cursor render JSON string"));
            }
            loop {
                let byte = *self
                    .bytes
                    .get(self.at)
                    .ok_or_else(|| RuntimeError::invalid("Incomplete Cursor render JSON string"))?;
                self.at += 1;
                if byte == b'"' {
                    return Ok(&self.bytes[start..self.at]);
                }
                if byte < 32 {
                    return Err(RuntimeError::invalid(
                        "Invalid Cursor render JSON string control",
                    ));
                }
                if byte == b'\\' {
                    let escaped = *self
                        .bytes
                        .get(self.at)
                        .ok_or_else(|| RuntimeError::invalid("Incomplete Cursor JSON escape"))?;
                    self.at += 1;
                    if escaped == b'u' {
                        let digits = self.bytes.get(self.at..self.at + 4).ok_or_else(|| {
                            RuntimeError::invalid("Incomplete Cursor JSON Unicode escape")
                        })?;
                        if !digits.iter().all(u8::is_ascii_hexdigit) {
                            return Err(RuntimeError::invalid(
                                "Invalid Cursor JSON Unicode escape",
                            ));
                        }
                        self.at += 4;
                    } else if !b"\"\\/bfnrt".contains(&escaped) {
                        return Err(RuntimeError::invalid("Invalid Cursor JSON escape"));
                    }
                }
            }
        }
        fn value(&mut self, wanted: Wanted, depth: usize) -> Result<RenderNode> {
            if depth > 1000 {
                return Err(RuntimeError::invalid("Cursor render JSON nesting limit"));
            }
            self.space();
            match self.bytes.get(self.at) {
                Some(b'"') => {
                    let bytes = self.string()?;
                    Ok(if matches!(wanted, Wanted::Text) {
                        serde_json::from_slice::<String>(bytes)
                            .map(RenderNode::Text)
                            .unwrap_or(RenderNode::Other)
                    } else {
                        RenderNode::Other
                    })
                }
                Some(b'{') => {
                    self.at += 1;
                    let mut values = std::collections::BTreeMap::new();
                    self.space();
                    if self.take(b'}') {
                        return Ok(RenderNode::Object(values));
                    }
                    loop {
                        self.space();
                        let key = serde_json::from_slice::<String>(self.string()?).ok();
                        self.space();
                        if !self.take(b':') {
                            return Err(RuntimeError::invalid("Invalid Cursor render JSON object"));
                        }
                        let next = match (wanted, key.as_deref()) {
                            (Wanted::Root, Some("capsule")) => Wanted::Capsule,
                            (Wanted::Root, Some("capsule_text")) => Wanted::Text,
                            (Wanted::Capsule, Some("gate")) => Wanted::Gate,
                            (Wanted::Gate, Some("mode" | "decision" | "reason")) => Wanted::Text,
                            _ => Wanted::Ignore,
                        };
                        let value = self.value(next, depth + 1)?;
                        if !matches!(next, Wanted::Ignore)
                            && let Some(key) = key
                        {
                            values.insert(key, value);
                        }
                        self.space();
                        if self.take(b'}') {
                            break;
                        }
                        if !self.take(b',') {
                            return Err(RuntimeError::invalid("Invalid Cursor render JSON object"));
                        }
                    }
                    Ok(
                        if matches!(wanted, Wanted::Root | Wanted::Capsule | Wanted::Gate) {
                            RenderNode::Object(values)
                        } else {
                            RenderNode::Other
                        },
                    )
                }
                Some(b'[') => {
                    self.at += 1;
                    self.space();
                    if self.take(b']') {
                        return Ok(RenderNode::Other);
                    }
                    loop {
                        self.value(Wanted::Ignore, depth + 1)?;
                        self.space();
                        if self.take(b']') {
                            break;
                        }
                        if !self.take(b',') {
                            return Err(RuntimeError::invalid("Invalid Cursor render JSON array"));
                        }
                    }
                    Ok(RenderNode::Other)
                }
                Some(_) => {
                    let start = self.at;
                    while self
                        .bytes
                        .get(self.at)
                        .is_some_and(|byte| !b" \t\r\n,]}".contains(byte))
                    {
                        self.at += 1;
                    }
                    let token = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| {
                        RuntimeError::invalid("Invalid Cursor render JSON primitive")
                    })?;
                    if !["null", "true", "false", "NaN", "Infinity", "-Infinity"].contains(&token)
                        && !render_number(token)
                    {
                        return Err(RuntimeError::invalid(
                            "Invalid Cursor render JSON primitive",
                        ));
                    }
                    Ok(RenderNode::Other)
                }
                None => Err(RuntimeError::invalid("Incomplete Cursor render JSON")),
            }
        }
    }
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = parser.value(Wanted::Root, 0)?;
    parser.space();
    if parser.at != parser.bytes.len() {
        return Err(RuntimeError::invalid("Trailing Cursor render JSON"));
    }
    let capsule = value
        .get("capsule_text")
        .and_then(RenderNode::text)
        .unwrap_or("")
        .into();
    let gate = value.get("capsule").and_then(|capsule| capsule.get("gate"));
    let field = |name| {
        gate.and_then(|gate| gate.get(name))
            .and_then(RenderNode::text)
    };
    let withheld = field("mode") == Some("enforce")
        && field("decision") == Some("skip")
        && !["no-relevant-match", "empty-after-filter"]
            .iter()
            .any(|reason| field("reason") == Some(reason));
    Ok(RenderReport { capsule, withheld })
}

fn receipt_surrogates(raw: &[u8]) -> Vec<u8> {
    fn unit(raw: &[u8], at: usize) -> Option<u16> {
        if raw.get(at..at + 2) != Some(b"\\u") {
            return None;
        }
        let bytes = raw.get(at + 2..at + 6)?;
        let text = std::str::from_utf8(bytes).ok()?;
        u16::from_str_radix(text, 16).ok()
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    let mut quoted = false;
    while at < raw.len() {
        let byte = raw[at];
        if byte == b'"' {
            quoted = !quoted;
            out.push(byte);
            at += 1;
            continue;
        }
        if quoted && byte == b'\\' {
            if let Some(first) = unit(raw, at) {
                if (0xd800..=0xdbff).contains(&first)
                    && unit(raw, at + 6).is_some_and(|second| (0xdc00..=0xdfff).contains(&second))
                {
                    out.extend_from_slice(&raw[at..at + 12]);
                    at += 12;
                    continue;
                }
                if (0xd800..=0xdfff).contains(&first) {
                    // Only root can survive schema validation with this sentinel.
                    // NUL cannot equal an actual OS root, so relocation stays required.
                    out.extend_from_slice(b"\\u0000");
                    at += 6;
                    continue;
                }
            }
            out.push(byte);
            at += 1;
            if let Some(byte) = raw.get(at) {
                out.push(*byte);
                at += 1;
            }
            continue;
        }
        out.push(byte);
        at += 1;
    }
    out
}
fn render_number(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut at = 0usize;
    if bytes.get(at) == Some(&b'-') {
        at += 1;
    }
    match bytes.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => {
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return false;
        }
    }
    if bytes
        .get(at)
        .is_some_and(|byte| matches!(byte, b'e' | b'E'))
    {
        at += 1;
        if bytes
            .get(at)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            at += 1;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return false;
        }
    }
    at == bytes.len()
}

/// Isolated branch identity for the Cursor topic cache. This is a current
/// cache scope, not an authority or source-custody proof. It never writes trust.
pub fn scope_branch(context: &RuntimeContext) -> Result<String> {
    const SELECTORS: &[&str] = &[
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
    ];
    if SELECTORS
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
    {
        return Err(RuntimeError::unavailable("Cursor topic branch unavailable"));
    }
    let project = &context.roots.project;
    let mut observed = git_read(
        project,
        &args(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
        true,
        None,
        Some(65536),
        Duration::from_secs(1),
    )?;
    if observed.code == 128 && protected_safe_root(project)? {
        observed = git_read(
            project,
            &args(&[
                "-c",
                &format!(
                    "safe.directory={}",
                    root_text(&std::fs::canonicalize(project)?)?
                ),
                "symbolic-ref",
                "--quiet",
                "--short",
                "HEAD",
            ]),
            true,
            None,
            Some(65536),
            Duration::from_secs(1),
        )?;
    }
    if observed.code != 0 || observed.stdout.is_empty() || observed.stdout.len() > 1024 {
        return Err(RuntimeError::unavailable("Cursor topic branch unavailable"));
    }
    let branch = std::str::from_utf8(&observed.stdout)
        .map_err(|_| RuntimeError::invalid("Cursor branch is not UTF-8"))?
        .trim_matches(crate::results::py_space);
    if branch.is_empty() || branch.contains(['\r', '\n', '\0']) {
        return Err(RuntimeError::unavailable("Cursor topic branch unavailable"));
    }
    Ok(branch.into())
}

/// Bounded ordinary protected-policy query shared by current source admission.
/// Every error is explicit source-git; callers must not reinterpret it as no ignores.
pub(crate) fn source_git_ignored_paths(
    root: &Path,
    paths: &[String],
    timeout: Duration,
) -> Result<BTreeSet<String>> {
    let operation = (|| -> Result<BTreeSet<String>> {
        if paths.is_empty() {
            return Ok(BTreeSet::new());
        }
        if paths
            .iter()
            .any(|path| path.is_empty() || path.contains('\0'))
        {
            return Err(RuntimeError::invalid("Invalid Git source path"));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| RuntimeError::invalid("Invalid source Git deadline"))?;
        let Some(before) = source_frame(root, deadline)? else {
            return Ok(BTreeSet::new());
        };
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let pinned = || {
            let mut gitdir = OsString::from("--git-dir=");
            gitdir.push(&before.admin);
            let mut worktree = OsString::from("--work-tree=");
            worktree.push(&before.top);
            vec![gitdir, worktree]
        };
        let mut command = args(&[
            "-c",
            &format!("core.hooksPath={null}"),
            "-c",
            "core.fsmonitor=false",
        ]);
        command.extend(pinned());
        command.push("check-ignore".into());
        let unindexed = paths.len() > 256;
        if unindexed {
            command.push("--no-index".into());
        }
        command.extend(args(&["--stdin", "-z"]));
        let mut payload = paths.join("\0").into_bytes();
        payload.push(0);
        let observed = git_read(
            root,
            &command,
            false,
            Some(&payload),
            Some(payload.len()),
            deadline.saturating_duration_since(Instant::now()),
        )?;
        if observed.code == 1 && observed.stdout.is_empty() {
            if source_frame(root, deadline)?.as_ref() != Some(&before) {
                return Err(RuntimeError::conflict("Git source frame changed"));
            }
            return Ok(BTreeSet::new());
        }
        if observed.code != 0 || observed.stdout.is_empty() || observed.stdout.last() != Some(&0) {
            return Err(RuntimeError::unavailable("Git source policy unavailable"));
        }
        let mut counts = BTreeMap::<String, usize>::new();
        for path in paths {
            *counts.entry(path.clone()).or_default() += 1;
        }
        let mut selected = BTreeSet::new();
        for path in observed.stdout[..observed.stdout.len() - 1].split(|byte| *byte == 0) {
            let path = std::str::from_utf8(path)
                .map_err(|_| RuntimeError::invalid("Git source path is not UTF-8"))?;
            let count = counts
                .get_mut(path)
                .filter(|count| **count > 0)
                .ok_or_else(|| RuntimeError::invalid("Unexpected Git source policy path"))?;
            *count -= 1;
            selected.insert(path.to_string());
        }
        if unindexed && !selected.is_empty() {
            let mut tracked = BTreeSet::new();
            let mut batch = Vec::new();
            let mut size = 0usize;
            for (index, path) in selected.iter().enumerate() {
                batch.push(path.clone());
                size += path.len() + 1;
                if size < 16000 && index + 1 < selected.len() {
                    continue;
                }
                let mut command = args(&[
                    "--literal-pathspecs",
                    "-c",
                    &format!("core.hooksPath={null}"),
                    "-c",
                    "core.fsmonitor=false",
                ]);
                command.extend(pinned());
                command.extend(args(&["ls-files", "-z", "--cached", "--"]));
                command.extend(batch.iter().map(OsString::from));
                let output = git_read(
                    root,
                    &command,
                    false,
                    None,
                    Some(size.saturating_mul(3)),
                    deadline.saturating_duration_since(Instant::now()),
                )?;
                if output.code != 0 || !output.stdout.is_empty() && output.stdout.last() != Some(&0)
                {
                    return Err(RuntimeError::unavailable(
                        "Git source tracked query unavailable",
                    ));
                }
                if !output.stdout.is_empty() {
                    for name in output.stdout[..output.stdout.len() - 1].split(|byte| *byte == 0) {
                        let name = std::str::from_utf8(name).map_err(|_| {
                            RuntimeError::invalid("Git tracked source path is not UTF-8")
                        })?;
                        if !selected.contains(name) {
                            return Err(RuntimeError::unavailable(
                                "Unexpected tracked source path",
                            ));
                        }
                        tracked.insert(name.to_string());
                    }
                }
                batch.clear();
                size = 0;
            }
            selected.retain(|path| !tracked.contains(path));
        }
        if source_frame(root, deadline)?.as_ref() != Some(&before) {
            return Err(RuntimeError::conflict("Git source frame changed"));
        }
        Ok(selected)
    })();
    operation.map_err(|_| {
        RuntimeError::coded(
            ErrorKind::Unavailable,
            "source-git",
            "Git source context unavailable",
        )
    })
}
