//! Session counters and writer leases. Paths retain the established POSIX cksum/MD5 keys.
use infra_core::{Result, RuntimeError, fs};
use serde_json::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
pub fn session(payload: &Value) -> String {
    let source = payload["session_id"]
        .as_str()
        .or_else(|| payload["conversation_id"].as_str())
        .unwrap_or("");
    let value: String = source
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(64)
        .collect();
    if value.is_empty() {
        "shared".into()
    } else {
        value
    }
}
pub fn uid() -> String {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() }.to_string()
    }
    #[cfg(not(unix))]
    {
        "0".into()
    }
}
pub fn owned(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if fs::linked(&metadata) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        true
    }
}
pub fn checksum(raw: &[u8]) -> u32 {
    let mut crc = 0u32;
    let mut feed = |byte: u8| {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x80000000 != 0 {
                (crc << 1) ^ 0x04c11db7
            } else {
                crc << 1
            };
        }
    };
    for byte in raw {
        feed(*byte);
    }
    let mut length = raw.len();
    while length != 0 {
        feed((length & 255) as u8);
        length >>= 8;
    }
    !crc
}
pub fn repo_key(root: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        checksum(root.as_os_str().as_bytes())
    }
    #[cfg(not(unix))]
    {
        checksum(root.to_string_lossy().as_bytes())
    }
}
pub fn directory(root: &Path) -> Result<PathBuf> {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = base.join(format!(
        "claude-loop-detection-{}-{}",
        uid(),
        repo_key(root)
    ));
    if !path.exists() {
        fs::ensure_directory(&path, true)?;
    }
    if !owned(&path) || !path.is_dir() {
        return Err(RuntimeError::policy("Counter directory is not owned"));
    }
    Ok(path)
}
pub fn md5(bytes: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let bits = (bytes.len() as u64).wrapping_mul(8);
    let mut input = bytes.to_vec();
    input.push(128);
    while input.len() % 64 != 56 {
        input.push(0);
    }
    input.extend_from_slice(&bits.to_le_bytes());
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in input.as_chunks::<64>().0 {
        let mut words = [0u32; 16];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            words[i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (state[0], state[1], state[2], state[3]);
        for i in 0..64 {
            let (f, g) = if i < 16 {
                ((b & c) | (!b & d), i)
            } else if i < 32 {
                ((d & b) | (!d & c), (5 * i + 1) % 16)
            } else if i < 48 {
                (b ^ c ^ d, (3 * i + 5) % 16)
            } else {
                (c ^ (b | !d), (7 * i) % 16)
            };
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(words[g])
                    .rotate_left(S[i]),
            );
            a = d;
            d = c;
            c = b;
            b = next;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    state
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn advance(
    root: &Path,
    payload: &Value,
    kind: &str,
    identity: &str,
    reset_after_edit: bool,
) -> Result<u32> {
    let folder = directory(root)?;
    let mut image = identity.as_bytes().to_vec();
    if kind == "edit" {
        image.push(b'\n');
    }
    let target = folder.join(format!("{kind}-{}-{}", session(payload), md5(&image)));
    if target.exists() && !owned(&target) {
        return Err(RuntimeError::policy("Counter file is not owned"));
    }
    let mut count = std::fs::read_to_string(&target)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| text.len() <= 9 && text.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|text| text.parse::<u32>().ok())
        .unwrap_or(0);
    if reset_after_edit && count > 0 {
        let modified = std::fs::metadata(&target)?.modified()?;
        if std::fs::read_dir(&folder)?
            .filter_map(std::result::Result::ok)
            .any(|entry| {
                entry.file_name().to_string_lossy().starts_with("edit-")
                    && owned(&entry.path())
                    && entry
                        .metadata()
                        .and_then(|meta| meta.modified())
                        .is_ok_and(|at| at > modified)
            })
        {
            count = 0;
        }
    }
    count = count.saturating_add(1);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut output = options.open(target)?;
    if !output.metadata()?.is_file() {
        return Err(RuntimeError::policy("Counter target is not a regular file"));
    }
    writeln!(output, "{count}")?;
    Ok(count)
}
pub fn reset(root: &Path, payload: &Value) -> Result<()> {
    let folder = directory(root)?;
    let session = session(payload);
    let now = SystemTime::now();
    for entry in std::fs::read_dir(folder)?.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if !owned(&path) || !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let expired = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > Duration::from_secs(86400));
        if name.starts_with(&format!("cmd-{session}-"))
            || name.starts_with(&format!("edit-{session}-"))
            || expired
        {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}
pub fn writer_folder() -> Result<PathBuf> {
    let path = std::env::var_os("SUBAGENT_WRITE_LOCK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    if !path.is_absolute() || !path.is_dir() {
        return Err(RuntimeError::invalid(
            "Write-agent lock directory configuration is invalid.",
        ));
    }
    Ok(path)
}
pub fn take_writer(root: &Path, host: &str, agent: &str) -> Result<Option<String>> {
    let folder = writer_folder()?;
    let key = repo_key(root);
    let name = format!(
        "{}-write-agent-lock-{key}",
        if host == "cursor" { "cursor" } else { "claude" }
    );
    let _guard = infra_core::native::DrainGuard::acquire(&folder, &format!("{name}.guard"))?
        .ok_or_else(|| RuntimeError::conflict("Write-agent spawning is already in progress"))?;
    let path = folder.join(name);
    if path.exists() && !owned(&path) {
        return Err(RuntimeError::policy("Write-agent lock is not owned"));
    }
    let minutes = std::env::var("SUBAGENT_WRITE_LOCK_TTL_MINUTES")
        .ok()
        .filter(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);
    if path.is_file() {
        let fresh = std::fs::metadata(&path)?
            .modified()
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .is_some_and(|age| age < Duration::from_secs(minutes.saturating_mul(60)));
        if fresh {
            let holder = fs::read_regular(&path)?;
            if !holder.is_empty() {
                return Ok(Some(
                    String::from_utf8_lossy(&holder).chars().take(64).collect(),
                ));
            }
        }
    }
    fs::atomic_write(&path, agent.as_bytes())?;
    Ok(None)
}
pub fn release_writer(root: &Path, agent: &str) -> Result<()> {
    let folder = writer_folder()?;
    for host in ["claude", "cursor"] {
        let name = format!("{host}-write-agent-lock-{}", repo_key(root));
        let path = folder.join(&name);
        let Some(_guard) =
            infra_core::native::DrainGuard::acquire(&folder, &format!("{name}.guard"))?
        else {
            continue;
        };
        if owned(&path) && fs::read_regular(&path).ok().as_deref() == Some(agent.as_bytes()) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}
