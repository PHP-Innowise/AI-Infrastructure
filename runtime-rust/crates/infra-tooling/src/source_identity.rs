//! Content identity of the sources the `infra-engine` executable compiles.
//!
//! `infra-cli`'s build script includes this file and embeds the digest of the
//! tree it compiles; `capabilities --json` reports it and assembly copies it
//! into `runtime-capabilities.json`. After a fast-forward the Harness reads
//! the clone again and restarts only on an engine that carries the same
//! digest, so a pull that changes no engine input needs no rebuild.
//!
//! The build script compiles this file on its own: std and `sha2` only.
//! Changing which files are read or how they are hashed is a new [`SCHEME`].
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

/// Names the input set and the digest computed over it.
pub const SCHEME: &str = "infra-engine-sources-v1";
const RUNTIME: &str = "runtime-rust";
const FILE_COUNT: usize = 20_000;
const BYTE_COUNT: u64 = 1024 * 1024 * 1024;
const DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub struct Identity {
    /// Lower-case hex SHA-256 over each input's repository path and digest.
    pub sha256: String,
    /// Repository-relative inputs, `/`-separated, in digest order.
    pub inputs: Vec<String>,
    /// The paths a change to the inputs touches: what a build script watches.
    pub watched: Vec<PathBuf>,
}

/// Reads the engine's compiled inputs under `repository`: the workspace lock
/// file and `.cargo/`, each package's `Cargo.toml`, `build.rs` and `src/`, and
/// every repository file those sources name in `include!`, `include_str!`,
/// `include_bytes!` or `#[path = "…"]`. Tests, examples and documentation are
/// not inputs; neither is any `target/` directory.
pub fn engine(repository: &Path) -> io::Result<Identity> {
    let mut tree = Tree {
        repository,
        files: BTreeMap::new(),
        sources: Vec::new(),
        watched: Vec::new(),
        bytes: 0,
    };
    tree.root(&format!("{RUNTIME}/Cargo.lock"))?;
    tree.root(&format!("{RUNTIME}/.cargo"))?;
    let mut packages = vec![RUNTIME.to_string()];
    for group in ["crates", "vendor"] {
        for name in names(&repository.join(RUNTIME).join(group))? {
            packages.push(format!("{RUNTIME}/{group}/{name}"));
        }
    }
    for package in packages {
        if !repository.join(&package).join("Cargo.toml").is_file() {
            continue;
        }
        for part in ["Cargo.toml", "build.rs", "src"] {
            tree.root(&format!("{package}/{part}"))?;
        }
    }
    while let Some((name, text)) = tree.sources.pop() {
        for literal in references(&text) {
            let Some(target) = resolve(&name, literal) else {
                continue;
            };
            let path = repository.join(&target);
            if tree.files.contains_key(&target) || !path.is_file() {
                continue;
            }
            tree.watched.push(path);
            tree.file(target)?;
        }
    }
    let mut hasher = Sha256::new();
    for (name, digest) in &tree.files {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(digest.as_bytes());
    }
    Ok(Identity {
        sha256: hex(&hasher.finalize()),
        inputs: tree.files.into_keys().collect(),
        watched: tree.watched,
    })
}

struct Tree<'a> {
    repository: &'a Path,
    files: BTreeMap<String, String>,
    sources: Vec<(String, String)>,
    watched: Vec<PathBuf>,
    bytes: u64,
}
impl Tree<'_> {
    fn root(&mut self, name: &str) -> io::Result<()> {
        let path = self.repository.join(name);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        self.watched.push(path);
        if metadata.is_dir() {
            self.directory(name, 0)
        } else {
            self.file(name.to_string())
        }
    }
    fn directory(&mut self, name: &str, depth: usize) -> io::Result<()> {
        if depth > DEPTH {
            return Err(io::Error::other(format!(
                "engine sources nest too deeply at {name}"
            )));
        }
        for entry in names(&self.repository.join(name))? {
            if entry == "target" || entry == ".git" {
                continue;
            }
            let child = format!("{name}/{entry}");
            if std::fs::metadata(self.repository.join(&child))?.is_dir() {
                self.directory(&child, depth + 1)?;
            } else {
                self.file(child)?;
            }
        }
        Ok(())
    }
    fn file(&mut self, name: String) -> io::Result<()> {
        let bytes = std::fs::read(self.repository.join(&name))?;
        self.bytes += bytes.len() as u64;
        if self.files.len() >= FILE_COUNT || self.bytes > BYTE_COUNT {
            return Err(io::Error::other("engine sources exceed their bound"));
        }
        if name.ends_with(".rs")
            && let Ok(text) = std::str::from_utf8(&bytes)
        {
            self.sources.push((name.clone(), text.to_string()));
        }
        self.files.insert(name, hex(&Sha256::digest(&bytes)));
        Ok(())
    }
}

/// Entry names of a directory in byte order; none when it does not exist.
fn names(directory: &Path) -> io::Result<Vec<String>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        names.push(
            name.into_string()
                .map_err(|_| io::Error::other("engine source name is not UTF-8"))?,
        );
    }
    names.sort();
    Ok(names)
}

/// Literal paths a Rust source names for the compiler to read.
fn references(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for (marker, opener) in [
        ("include!", '('),
        ("include_str!", '('),
        ("include_bytes!", '('),
        ("#[path", '='),
    ] {
        for (at, _) in text.match_indices(marker) {
            let Some(rest) = text[at + marker.len()..].trim_start().strip_prefix(opener) else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix('"') else {
                continue;
            };
            if let Some(end) = rest.find('"')
                && !rest[..end].contains('\\')
            {
                found.push(&rest[..end]);
            }
        }
    }
    found
}

/// A literal resolved against the directory of the file that names it; none
/// when it is absolute or leaves the repository.
fn resolve(file: &str, literal: &str) -> Option<String> {
    if literal.starts_with('/') || literal.contains(':') {
        return None;
    }
    let mut parts = file.split('/').collect::<Vec<_>>();
    parts.pop();
    for part in literal.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, name: &str, text: &str) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn repository() -> crate::temp::Temporary {
        let root = crate::temp::Temporary::new("source-identity").unwrap();
        let path = root.path();
        write(path, "runtime-rust/Cargo.toml", "[workspace]\n");
        write(path, "runtime-rust/Cargo.lock", "version = 4\n");
        write(path, "runtime-rust/README.md", "# Runtime\n");
        write(
            path,
            "runtime-rust/crates/infra-a/Cargo.toml",
            "[package]\n",
        );
        write(
            path,
            "runtime-rust/crates/infra-a/src/lib.rs",
            "mod nested;\nconst POLICY: &str = include_str!(\"../../../../PHP Core/.mcp.json\");\nconst RULES: &[u8] = include_bytes! ( \"../resources/rules.json\" );\n",
        );
        write(
            path,
            "runtime-rust/crates/infra-a/src/nested.rs",
            "#[path = \"../../../../harness/web/shared.rs\"]\nmod shared;\n",
        );
        write(
            path,
            "runtime-rust/crates/infra-a/resources/rules.json",
            "{}",
        );
        write(
            path,
            "runtime-rust/crates/infra-a/resources/unused.json",
            "{}",
        );
        write(
            path,
            "runtime-rust/crates/infra-a/tests/contract.rs",
            "#[test]\nfn t() {}\n",
        );
        write(path, "runtime-rust/vendor/numfmt/Cargo.toml", "[package]\n");
        write(
            path,
            "runtime-rust/vendor/numfmt/src/lib.rs",
            "pub fn f() {}\n",
        );
        write(path, "runtime-rust/vendor/numfmt/README.md", "upstream\n");
        write(path, "runtime-rust/target/release/infra-engine", "binary");
        write(path, "PHP Core/.mcp.json", "{\"mcpServers\":{}}");
        write(
            path,
            "harness/web/shared.rs",
            "const BOOT: &str = include_str!(\"boot.js\");\n",
        );
        write(
            path,
            "harness/web/boot.js",
            "import init from '/assets/infra_ui.js';\n",
        );
        write(path, "README.md", "# Repository\n");
        root
    }

    #[test]
    fn inputs_are_the_compiled_trees_and_every_file_they_include() {
        let root = repository();
        let identity = engine(root.path()).unwrap();
        assert_eq!(
            identity.inputs,
            [
                "PHP Core/.mcp.json",
                "harness/web/boot.js",
                "harness/web/shared.rs",
                "runtime-rust/Cargo.lock",
                "runtime-rust/Cargo.toml",
                "runtime-rust/crates/infra-a/Cargo.toml",
                "runtime-rust/crates/infra-a/resources/rules.json",
                "runtime-rust/crates/infra-a/src/lib.rs",
                "runtime-rust/crates/infra-a/src/nested.rs",
                "runtime-rust/vendor/numfmt/Cargo.toml",
                "runtime-rust/vendor/numfmt/src/lib.rs",
            ]
        );
        assert_eq!(identity.sha256.len(), 64);
        for watched in [
            "runtime-rust/Cargo.lock",
            "runtime-rust/crates/infra-a/src",
            "runtime-rust/crates/infra-a/resources/rules.json",
            "PHP Core/.mcp.json",
            "harness/web/boot.js",
        ] {
            assert!(
                identity.watched.contains(&root.path().join(watched)),
                "{watched} is not watched"
            );
        }
        assert!(
            !identity.watched.iter().any(|path| path
                .starts_with(root.path().join("runtime-rust/target"))
                || path.ends_with("tests")),
            "build output or tests are watched"
        );
    }

    #[test]
    fn only_a_change_to_an_input_changes_the_digest() {
        let root = repository();
        let first = engine(root.path()).unwrap().sha256;
        for (name, text) in [
            ("README.md", "# Repository, edited\n"),
            ("runtime-rust/README.md", "# Runtime, edited\n"),
            (
                "runtime-rust/crates/infra-a/tests/contract.rs",
                "#[test]\nfn u() {}\n",
            ),
            ("runtime-rust/crates/infra-a/resources/unused.json", "[]"),
            ("runtime-rust/vendor/numfmt/README.md", "patched\n"),
            ("runtime-rust/target/release/infra-engine", "rebuilt"),
        ] {
            write(root.path(), name, text);
            assert_eq!(engine(root.path()).unwrap().sha256, first, "{name}");
        }
        let mut seen = vec![first];
        for (name, text) in [
            ("PHP Core/.mcp.json", "{\"mcpServers\":{\"x\":{}}}"),
            (
                "harness/web/boot.js",
                "import init from '/assets/other.js';\n",
            ),
            ("runtime-rust/crates/infra-a/resources/rules.json", "[]"),
            (
                "runtime-rust/crates/infra-a/src/added.rs",
                "pub fn added() {}\n",
            ),
            ("runtime-rust/Cargo.lock", "version = 4\n# changed\n"),
            ("runtime-rust/vendor/numfmt/src/lib.rs", "pub fn g() {}\n"),
            ("runtime-rust/crates/infra-a/build.rs", "fn main() {}\n"),
        ] {
            write(root.path(), name, text);
            let digest = engine(root.path()).unwrap().sha256;
            assert!(!seen.contains(&digest), "{name} did not change the digest");
            seen.push(digest);
        }
    }

    #[test]
    fn literals_resolve_inside_the_repository_only() {
        assert_eq!(
            resolve(
                "runtime-rust/crates/infra-a/src/lib.rs",
                "../../../../PHP Core/.mcp.json"
            )
            .as_deref(),
            Some("PHP Core/.mcp.json")
        );
        assert_eq!(
            resolve("runtime-rust/crates/infra-a/src/lib.rs", "./x/../y.rs").as_deref(),
            Some("runtime-rust/crates/infra-a/src/y.rs")
        );
        assert_eq!(resolve("runtime-rust/src/lib.rs", "../../../outside"), None);
        assert_eq!(resolve("runtime-rust/src/lib.rs", "/etc/passwd"), None);
        assert_eq!(resolve("runtime-rust/src/lib.rs", "C:/Windows"), None);
        // A computed path or an escape is not a literal path; it is skipped.
        assert_eq!(
            references(
                "include_str!(\"a\"); include_bytes!(\n  \"b\"); include!(concat!(env!(\"X\"), \"c\"));\n#[path = \"d.rs\"]\nmod d;\ninclude_str!(\"e\\\\f\");"
            ),
            ["a", "b", "d.rs"]
        );
    }
}
