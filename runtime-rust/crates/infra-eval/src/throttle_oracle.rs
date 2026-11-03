//! Opt-in EntryFetchThrottle artifact oracle v2. Its nested PHP executions see only public task bytes.
use crate::{Result, artifact};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
const SOURCE: &str = "packages/website-public-api/app/src/Service/EntryFetchThrottle.php";
const TEST: &str = "packages/website-public-api/app/tests/Unit/Service/EntryFetchThrottleTest.php";
const PROFILE: &str = "entryfetch-throttle-oracle-v2";
const ALLOWED: &str = "return $this->consume('car_financing_entry_fetch_' . sha1($this->callerId()), self::MAX_ATTEMPTS)\n            && $this->consume('car_financing_entry_fetch_global', self::MAX_GLOBAL_ATTEMPTS);";
const CALLER: &str =
    "$this->consume('car_financing_entry_fetch_' . sha1($this->callerId()), self::MAX_ATTEMPTS)";
const GLOBAL: &str =
    "$this->consume('car_financing_entry_fetch_global', self::MAX_GLOBAL_ATTEMPTS)";
const DENY: &str = "if ($attempts >= $maxAttempts) {\n            return false;\n        }";
const SET: &str = "$item->set([$attempts + 1, $windowStartedAt]);";
const TTL: &str = "$item->expiresAfter($remaining);";
const DISABLED: &str =
    "sleep,usleep,time_nanosleep,time_sleep_until,time,microtime,hrtime,gettimeofday,date,gmdate";
fn unknown() -> infra_core::RuntimeError {
    artifact::invalid("B oracle infrastructure unavailable")
}
fn replace(source: &str, old: &str, new: &str) -> Result<Vec<u8>> {
    if source.matches(old).count() != 1 {
        return Err(unknown());
    }
    Ok(source.replace(old, new).into_bytes())
}
pub fn variants(source: &[u8]) -> Result<Vec<(String, Vec<u8>, bool)>> {
    let s = std::str::from_utf8(source).map_err(|_| unknown())?;
    let mut controls = vec![];
    macro_rules! add {
        ($name:expr,$old:expr,$new:expr) => {
            controls.push(($name.to_string(), replace(s, $old, $new)?, false));
        };
    }
    for (name, old, new) in [
        (
            "caller-cap-59",
            "private const MAX_ATTEMPTS = 60;",
            "private const MAX_ATTEMPTS = 59;",
        ),
        (
            "caller-cap-61",
            "private const MAX_ATTEMPTS = 60;",
            "private const MAX_ATTEMPTS = 61;",
        ),
        (
            "global-cap-5999",
            "private const MAX_GLOBAL_ATTEMPTS = 6000;",
            "private const MAX_GLOBAL_ATTEMPTS = 5999;",
        ),
        (
            "global-cap-6001",
            "private const MAX_GLOBAL_ATTEMPTS = 6000;",
            "private const MAX_GLOBAL_ATTEMPTS = 6001;",
        ),
        (
            "window-59",
            "private const WINDOW_SECONDS = 60;",
            "private const WINDOW_SECONDS = 59;",
        ),
        (
            "window-61",
            "private const WINDOW_SECONDS = 60;",
            "private const WINDOW_SECONDS = 61;",
        ),
        ("allow-all", ALLOWED, "return true;"),
        ("deny-all", ALLOWED, "return false;"),
        (
            "shared-all-callers",
            "sha1($this->callerId())",
            "sha1('all-callers')",
        ),
        (
            "request-object-identity",
            "sha1($this->callerId())",
            "sha1((string) spl_object_id($this->requestStack->getCurrentRequest()))",
        ),
        (
            "instance-local-identity",
            "sha1($this->callerId())",
            "sha1($this->callerId() . ':' . spl_object_id($this))",
        ),
        (
            "denial-increments",
            DENY,
            "if ($attempts >= $maxAttempts) {\n            $item->set([$attempts + 1, $windowStartedAt]);\n            $item->expiresAfter($remaining);\n            $this->cache->save($item);\n            return false;\n        }",
        ),
        (
            "denial-resaves",
            DENY,
            "if ($attempts >= $maxAttempts) {\n            $item->set([$attempts, $windowStartedAt]);\n            $item->expiresAfter($remaining);\n            $this->cache->save($item);\n            return false;\n        }",
        ),
        (
            "global-denial-increments",
            DENY,
            "if ($attempts >= $maxAttempts) {\n            if ($key === 'car_financing_entry_fetch_global') {\n                $item->set([$attempts + 1, $windowStartedAt]);\n                $item->expiresAfter($remaining);\n                $this->cache->save($item);\n            }\n            return false;\n        }",
        ),
        (
            "global-denial-resaves",
            DENY,
            "if ($attempts >= $maxAttempts) {\n            if ($key === 'car_financing_entry_fetch_global') {\n                $item->set([$attempts, $windowStartedAt]);\n                $item->expiresAfter($remaining);\n                $this->cache->save($item);\n            }\n            return false;\n        }",
        ),
        (
            "ttl-only-first-save",
            TTL,
            "if ($attempts === 0) { $item->expiresAfter($remaining); }",
        ),
        (
            "ttl-always-full-window",
            TTL,
            "$item->expiresAfter(self::WINDOW_SECONDS);",
        ),
        (
            "sliding-window",
            SET,
            "$windowStartedAt = time();\n        $remaining = self::WINDOW_SECONDS;\n        $item->set([$attempts + 1, $windowStartedAt]);",
        ),
        (
            "saved-start-changes",
            SET,
            "$item->set([$attempts + 1, time()]);",
        ),
        (
            "counter-stays-one",
            SET,
            "$item->set([1, $windowStartedAt]);",
        ),
        (
            "reset-after-boundary",
            "if ($remaining <= 0) {",
            "if ($remaining < 0) {",
        ),
        (
            "no-hit-expiry-reset",
            "if ($remaining <= 0) {",
            "if (false && $remaining <= 0) {",
        ),
        (
            "ignore-valid-hit-pair",
            "$item->isHit() && is_array($item->get())",
            "false",
        ),
        (
            "nonarray-hit-starts-used",
            ": [0, time()];",
            ": [$item->isHit() ? 59 : 0, time()];",
        ),
        ("miss-starts-two", ": [0, time()];", ": [1, time()];"),
        (
            "separate-fallback-identities",
            "?? 'unknown-caller'",
            "?? ($this->requestStack->getCurrentRequest() === null ? 'no-request' : 'no-ip')",
        ),
    ] {
        add!(name, old, new);
    }
    add!("no-global-charge", ALLOWED, &format!("return {CALLER};"));
    add!("no-caller-charge", ALLOWED, &format!("return {GLOBAL};"));
    add!(
        "global-first",
        ALLOWED,
        &format!("return {GLOBAL} && {CALLER};")
    );
    add!(
        "eager-global-on-caller-denial",
        ALLOWED,
        &format!("return (bool) ({CALLER} & {GLOBAL});")
    );
    add!(
        "fallback-unlimited",
        ALLOWED,
        &format!(
            "if ($this->requestStack->getCurrentRequest()?->getClientIp() === null) {{ return true; }}\n        {ALLOWED}"
        )
    );
    add!(
        "fallback-skips-global",
        ALLOWED,
        &format!(
            "return {CALLER} && ($this->requestStack->getCurrentRequest()?->getClientIp() === null || {GLOBAL});"
        )
    );
    for (bucket, condition) in [
        ("global", "$key === 'car_financing_entry_fetch_global'"),
        ("caller", "$key !== 'car_financing_entry_fetch_global'"),
    ] {
        for (name, old, new) in [
            (
                format!("{bucket}-ttl-never-set"),
                TTL,
                format!("if (!({condition})) {{ $item->expiresAfter($remaining); }}"),
            ),
            (
                format!("{bucket}-ttl-only-first"),
                TTL,
                format!(
                    "if (!({condition}) || $attempts === 0) {{ $item->expiresAfter($remaining); }}"
                ),
            ),
            (
                format!("{bucket}-ttl-always-full"),
                TTL,
                format!("$item->expiresAfter(({condition}) ? self::WINDOW_SECONDS : $remaining);"),
            ),
            (
                format!("{bucket}-saved-start-slides"),
                SET,
                format!("$item->set([$attempts + 1, ({condition}) ? time() : $windowStartedAt]);"),
            ),
            (
                format!("{bucket}-late-boundary-reset"),
                "if ($remaining <= 0) {",
                format!("if ($remaining < 0 || ($remaining === 0 && !({condition}))) {{"),
            ),
            (
                format!("{bucket}-no-hit-expiry-reset"),
                "if ($remaining <= 0) {",
                format!("if ($remaining <= 0 && !({condition})) {{"),
            ),
            (
                format!("{bucket}-nonarray-starts-used"),
                ": [0, time()];",
                format!(": [($item->isHit() && ({condition})) ? 59 : 0, time()];"),
            ),
            (
                format!("{bucket}-miss-starts-two"),
                ": [0, time()];",
                format!(": [(! $item->isHit() && ({condition})) ? 1 : 0, time()];"),
            ),
        ] {
            add!(name, old, &new);
        }
    }
    let memo=s.replace("private const WINDOW_SECONDS = 60;","private array $seen = [];\n    private const WINDOW_SECONDS = 60;").replace(ALLOWED,&format!("$request = $this->requestStack->getCurrentRequest();\n        if ($request !== null) {{\n            $id = spl_object_id($request);\n            if (isset($this->seen[$id])) {{ return true; }}\n            $this->seen[$id] = true;\n        }}\n        {ALLOWED}"));
    controls.push(("memoize-one-request".into(), memo.into_bytes(), false));
    let coupled=s.replace("private const WINDOW_SECONDS = 60;","private ?int $sharedStart = null;\n    private const WINDOW_SECONDS = 60;").replace("$remaining = self::WINDOW_SECONDS - (time() - $windowStartedAt);","$this->sharedStart ??= $windowStartedAt;\n        $windowStartedAt = $this->sharedStart;\n        $remaining = self::WINDOW_SECONDS - (time() - $windowStartedAt);");
    controls.push((
        "coupled-bucket-window-starts".into(),
        coupled.into_bytes(),
        false,
    ));
    let order = [
        "caller-cap-59",
        "caller-cap-61",
        "global-cap-5999",
        "global-cap-6001",
        "window-59",
        "window-61",
        "allow-all",
        "deny-all",
        "no-global-charge",
        "no-caller-charge",
        "global-first",
        "eager-global-on-caller-denial",
        "shared-all-callers",
        "request-object-identity",
        "instance-local-identity",
        "denial-increments",
        "denial-resaves",
        "global-denial-increments",
        "global-denial-resaves",
        "ttl-only-first-save",
        "ttl-always-full-window",
        "sliding-window",
        "saved-start-changes",
        "counter-stays-one",
        "reset-after-boundary",
        "no-hit-expiry-reset",
        "ignore-valid-hit-pair",
        "nonarray-hit-starts-used",
        "miss-starts-two",
        "separate-fallback-identities",
        "fallback-unlimited",
        "fallback-skips-global",
    ];
    let original_positions = controls
        .iter()
        .enumerate()
        .map(|(i, (name, _, _))| (name.clone(), i))
        .collect::<std::collections::BTreeMap<_, _>>();
    controls.sort_by_key(|(name, _, _)| {
        order
            .iter()
            .position(|v| *v == name)
            .unwrap_or(order.len() + original_positions[name])
    });
    let comments = regex::Regex::new(r"(?s)/\*.*?\*/")
        .map_err(|_| unknown())?
        .replace_all(s, "")
        .into_owned();
    let equivalent = regex::Regex::new(r"(?m)^\s*#[^\n]*$")
        .map_err(|_| unknown())?
        .replace_all(&comments, "")
        .replace('\n', "\n\n")
        .into_bytes();
    let mut out = vec![
        ("original".into(), source.to_vec(), true),
        ("equivalent-comments-whitespace".into(), equivalent, true),
    ];
    out.extend(controls);
    Ok(out)
}
#[cfg(target_os = "linux")]
fn execute(argv: &[String], marker: Option<&[u8]>) -> Result<Value> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::{CommandExt, ExitStatusExt},
    };
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut fd = [0; 2];
    if unsafe { libc::pipe2(fd.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(unknown());
    }
    let mut read = unsafe { std::fs::File::from_raw_fd(fd[0]) };
    let write = unsafe { std::fs::File::from_raw_fd(fd[1]) };
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(write.try_clone()?)
        .stderr(write.try_clone()?);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = command.spawn()?;
    // Command retains the parent's cloned writers until it is dropped.
    drop(command);
    drop(write);
    unsafe {
        libc::fcntl(read.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let cap = 65536 + marker.map(<[u8]>::len).unwrap_or(0);
    let mut raw = vec![];
    let mut reason = None;
    loop {
        let mut buf = [0; 8192];
        match read.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > cap {
                    reason = Some("output-cap");
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                reason = Some("io");
                break;
            }
        }
        if Instant::now() >= deadline || crate::cancel::interrupted() {
            reason = Some("timeout");
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let end = Instant::now() + Duration::from_secs(1);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if reason.is_some() || Instant::now() >= end {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let status = child.wait()?;
            if reason.is_none() {
                return Err(unknown());
            }
            break status;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    raw.truncate(cap);
    if let Some(marker) = marker {
        if !raw.starts_with(marker) {
            return Err(unknown());
        }
        raw.drain(..marker.len());
    }
    Ok(
        json!({"code":status.code().unwrap_or_else(||-status.signal().unwrap_or(1)),"output":String::from_utf8_lossy(&raw),"limit":reason}),
    )
}
#[cfg(not(target_os = "linux"))]
fn execute(_: &[String], _: Option<&[u8]>) -> Result<Value> {
    Err(unknown())
}
fn system_links() -> Result<Vec<String>> {
    let mut args = vec![];
    for name in ["bin", "lib", "lib64"] {
        let path = PathBuf::from("/").join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            args.extend([
                "--symlink".into(),
                std::fs::read_link(&path)?.to_string_lossy().into(),
                format!("/{name}"),
            ]);
        } else if path.is_dir() {
            args.extend(["--ro-bind".into(), format!("/{name}"), format!("/{name}")]);
        }
    }
    Ok(args)
}
pub struct Oracle {
    pub php: PathBuf,
}
impl Oracle {
    fn nested(
        &self,
        script: &[u8],
        source: &[u8],
        test: &[u8],
        argument: &str,
        lint: bool,
        shadow: Option<&[u8]>,
    ) -> Result<Value> {
        let work = artifact::Scratch::new(&std::env::temp_dir(), "b-oracle-nested")?;
        let program = work.0.join("program.php");
        let source_file = work.0.join("source.php");
        let test_file = work.0.join("test.php");
        artifact::fresh_file(&program, script, 0o444)?;
        artifact::fresh_file(
            &source_file,
            source,
            if argument == "unreadable" { 0 } else { 0o444 },
        )?;
        artifact::fresh_file(&test_file, test, 0o444)?;
        let mut command = vec![
            "/usr/bin/bwrap".into(),
            "--die-with-parent".into(),
            "--new-session".into(),
            "--unshare-pid".into(),
            "--as-pid-1".into(),
            "--unshare-net".into(),
            "--unshare-ipc".into(),
            "--unshare-uts".into(),
            "--tmpfs".into(),
            "/".into(),
            "--ro-bind".into(),
            "/usr".into(),
            "/usr".into(),
        ];
        command.extend(system_links()?);
        command.extend([
            "--proc".into(),
            "/proc".into(),
            "--dev".into(),
            "/dev".into(),
            "--tmpfs".into(),
            "/tmp".into(),
            "--dir".into(),
            "/work".into(),
            "--ro-bind".into(),
            program.to_string_lossy().into(),
            "/work/verification/throttle_regression.php".into(),
            "--ro-bind".into(),
            source_file.to_string_lossy().into(),
            format!("/work/{SOURCE}"),
            "--ro-bind".into(),
            test_file.to_string_lossy().into(),
            format!("/work/{TEST}"),
        ]);
        if argument == "bare-collision" {
            let shadow_file = work.0.join("shadow.php");
            artifact::fresh_file(&shadow_file, shadow.ok_or_else(unknown)?, 0o444)?;
            command.extend([
                "--ro-bind".into(),
                source_file.to_string_lossy().into(),
                "/work/EntryFetchThrottle.php".into(),
                "--ro-bind".into(),
                shadow_file.to_string_lossy().into(),
                "/shadow/EntryFetchThrottle.php".into(),
            ]);
        }
        command.extend([
            "--chdir".into(),
            "/work".into(),
            "--clearenv".into(),
            "--setenv".into(),
            "PATH".into(),
            "/usr/bin:/bin".into(),
            "--setenv".into(),
            "LANG".into(),
            "C.UTF-8".into(),
            "--setenv".into(),
            "TZ".into(),
            "UTC".into(),
            "--".into(),
            self.php.to_string_lossy().into(),
            "-n".into(),
            "-d".into(),
            "zend.assertions=-1".into(),
            "-d".into(),
            "memory_limit=128M".into(),
            "-d".into(),
            "max_execution_time=4".into(),
            "-d".into(),
            format!("disable_functions={DISABLED}"),
        ]);
        if argument == "bare-collision" {
            command.extend(["-d".into(), "include_path=/shadow:.".into()]);
        }
        if lint {
            command.extend(["-l".into(), format!("/work/{SOURCE}")]);
            return execute(&command, None);
        }
        let marker = format!("B-START:{}\n", uuid::Uuid::new_v4().simple());
        let bootstrap = format!(
            "if (fwrite(STDOUT, {}) !== {}) {{ exit(125); }} fflush(STDOUT); array_shift($argv); $argc = count($argv); $_SERVER[\"argv\"] = $argv; $_SERVER[\"argc\"] = $argc; $_SERVER[\"SCRIPT_FILENAME\"] = $argv[0]; $_SERVER[\"PHP_SELF\"] = $argv[0]; $_SERVER[\"SCRIPT_NAME\"] = $argv[0]; require $argv[0];",
            serde_json::to_string(&marker)?,
            marker.len()
        );
        command.extend([
            "-r".into(),
            bootstrap,
            "--".into(),
            "/work/verification/throttle_regression.php".into(),
        ]);
        match argument {
            "relative" | "unreadable" => command.push(SOURCE.into()),
            "bare-collision" => command.push("EntryFetchThrottle.php".into()),
            "absolute" => command.push(format!("/work/{SOURCE}")),
            "missing-file" => command.push("missing/EntryFetchThrottle.php".into()),
            "missing-argument" => {}
            _ => return Err(unknown()),
        }
        execute(&command, Some(marker.as_bytes()))
    }
    fn probe(&self, source: &[u8], test: &[u8]) -> Result<Value> {
        let sentinels = artifact::Scratch::new(&std::env::temp_dir(), "b-oracle-private")?;
        let private = sentinels.0.join("private-label-sentinel");
        artifact::fresh_file(&private, b"not visible to candidate", 0o600)?;
        let parent = private.parent().ok_or_else(unknown)?;
        let probe = format!(
            "<?php\n$source={};$result=['privateDenied'=>!file_exists({}),'homeDenied'=>!file_exists('/home'),'referenceDenied'=>!file_exists({}),'outerArgsDenied'=>!str_contains(file_get_contents('/proc/1/cmdline'),{}),'sourceReadonly'=>@file_put_contents($source,'tampered')===false,'testReadonly'=>@file_put_contents({},'tampered')===false,'pid'=>getmypid(),'net'=>readlink('/proc/self/ns/net'),'sourceDigest'=>hash_file('sha256',$source)];echo json_encode($result);",
            serde_json::to_string(&format!("/work/{SOURCE}"))?,
            serde_json::to_string(&private.to_string_lossy())?,
            serde_json::to_string(&parent.join("reference.php").to_string_lossy())?,
            serde_json::to_string(&parent.to_string_lossy())?,
            serde_json::to_string(&format!("/work/{TEST}"))?
        );
        let result = self.nested(probe.as_bytes(), source, test, "relative", false, None)?;
        if result["code"] != 0 || !result["limit"].is_null() {
            return Err(unknown());
        }
        let receipt = artifact::strict(result["output"].as_str().unwrap_or("").as_bytes())?;
        let outer = std::fs::read_link("/proc/self/ns/net")?
            .to_string_lossy()
            .into_owned();
        if ![
            "privateDenied",
            "homeDenied",
            "referenceDenied",
            "outerArgsDenied",
            "sourceReadonly",
            "testReadonly",
        ]
        .iter()
        .all(|k| receipt[*k] == true)
            || receipt["pid"] != 1
            || receipt["sourceDigest"] != artifact::sha(source)
            || receipt["net"] == outer
        {
            return Err(unknown());
        }
        Ok(receipt)
    }
    pub fn evaluate(&self, candidate: &Path, source: &[u8], test: &[u8]) -> Result<bool> {
        self.probe(source, test)?;
        let script = match artifact::read_file(
            &candidate.join("verification/throttle_regression.php"),
            1024 * 1024,
        ) {
            Ok(r) => r.0,
            Err(_) => return Ok(false),
        };
        for (name, expected) in [(SOURCE, source), (TEST, test)] {
            if !artifact::read_file(&candidate.join(name), 512 * 1024 * 1024)
                .is_ok_and(|r| r.0 == expected)
            {
                return Ok(false);
            }
        }
        let variants = variants(source)?;
        let broken = &variants
            .iter()
            .find(|v| v.0 == "allow-all")
            .ok_or_else(unknown)?
            .1;
        for (selected, shadow, expected) in [
            (source, broken.as_slice(), true),
            (broken.as_slice(), source, false),
        ] {
            let r = self.nested(
                &script,
                selected,
                test,
                "bare-collision",
                false,
                Some(shadow),
            )?;
            if !(if expected {
                success(&r)
            } else {
                meaningful(&r)
            }) {
                return Ok(false);
            }
        }
        let mut success_output = None::<String>;
        for (_name, image, expected) in variants {
            let lint = self.nested(b"<?php", &image, test, "relative", true, None)?;
            if !success(&lint) {
                return Err(unknown());
            }
            let result = self.nested(&script, &image, test, "relative", false, None)?;
            let mut passed = if expected {
                success(&result)
            } else {
                meaningful(&result)
            };
            if expected && success_output.is_none() {
                success_output = result["output"].as_str().map(String::from);
            }
            if !expected && result["output"].as_str() == success_output.as_deref() {
                passed = false;
            }
            if !passed {
                return Ok(false);
            }
        }
        for arg in ["absolute", "missing-argument", "missing-file", "unreadable"] {
            let result = self.nested(&script, source, test, arg, false, None)?;
            let passed = if arg == "absolute" {
                success(&result)
            } else {
                meaningful(&result) && result["output"].as_str() != success_output.as_deref()
            };
            if !passed {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
fn success(v: &Value) -> bool {
    v["code"] == 0 && v["limit"].is_null()
}
fn meaningful(v: &Value) -> bool {
    v["code"] != 0
        && v["limit"].is_null()
        && v["output"].as_str().is_some_and(|s| {
            s.trim().chars().count() >= 4
                && !regex::Regex::new(r"(?:^|\n)(?:PHP )?Parse error:").is_ok_and(|r| r.is_match(s))
        })
}
pub fn run(v: &Value) -> Result<Value> {
    let result = (|| -> Result<Value> {
        let source = artifact::read_file(
            Path::new(v["source"].as_str().ok_or_else(unknown)?),
            512 * 1024 * 1024,
        )?
        .0;
        let test = artifact::read_file(
            Path::new(v["public_test"].as_str().ok_or_else(unknown)?),
            512 * 1024 * 1024,
        )?
        .0;
        let oracle = Oracle {
            php: std::fs::canonicalize(v["php"].as_str().unwrap_or("/usr/bin/php"))?,
        };
        let correct = oracle.evaluate(&std::env::current_dir()?, &source, &test)?;
        Ok(json!({"schema":"task-outcome-verifier-v1","completed":true,"correct":correct}))
    })();
    match result {
        Ok(result) => Ok(
            json!({"_cli":{"code":0,"stdout":format!("{}\n",serde_json::to_string(&result)?),"stderr":""},"result":result}),
        ),
        Err(_) => Ok(
            json!({"_cli":{"code":70,"stdout":"","stderr":"B oracle infrastructure unavailable\n"},"result":{"profile":PROFILE,"status":"unavailable","correct":null}}),
        ),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod execute_pipe_tests {
    use super::*;

    #[test]
    fn zero_exit_reaches_pipe_eof_without_false_timeout() {
        let result = execute(&["/usr/bin/true".into()], None).unwrap();
        assert_eq!(result["code"], 0);
        assert!(result["limit"].is_null(), "{result}");
        assert_eq!(result["output"], "");
    }

    #[test]
    fn nonzero_exit_preserves_status_without_false_timeout() {
        let result = execute(&["/usr/bin/false".into()], None).unwrap();
        assert_eq!(result["code"], 1);
        assert!(result["limit"].is_null(), "{result}");
        assert_eq!(result["output"], "");
    }

    #[test]
    fn startup_marker_is_removed_and_remaining_output_is_preserved() {
        let marker = b"ORACLE-PIPE-START\n";
        let result = execute(
            &[
                "/usr/bin/printf".into(),
                "ORACLE-PIPE-START\nkept-output\n".into(),
            ],
            Some(marker),
        )
        .unwrap();
        assert_eq!(result["code"], 0);
        assert!(result["limit"].is_null(), "{result}");
        assert_eq!(result["output"], "kept-output\n");
    }
}
