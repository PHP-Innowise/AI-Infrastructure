//! Which links between Rust sources the build rules out.
//!
//! A code graph is made by matching names, and across crates a name matches
//! what its user cannot reach. Cargo knows what a crate can reach: itself,
//! the packages its manifest names, and what those are built with in turn.
//! This module reads the manifests of a tree and the places where its
//! sources take sources of other folders, and says of two files whether a
//! link from one to the other cannot be. Where it is not certain it says
//! that the link can be, so nothing true is left out of a map.
use crate::graph_map::{SOURCE_LIMIT, quiet_end};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::LazyLock,
};
/// No crate: a file that is no Rust source, or lies under no package.
const NONE: u32 = u32::MAX;
/// What is kept of a Rust source where the source itself is not, in a file
/// named after it with this ending: see `modules`.
pub const MODULES: &str = "modules";
/// One dependency of a Cargo manifest: the key it stands under, the package
/// where the key is another name for it, and whether the workspace says the
/// rest.
#[derive(Debug, Default, PartialEq)]
struct Needed {
    key: String,
    package: Option<String>,
    workspace: bool,
}
/// What a Cargo manifest says of a crate, as far as it tells which crates
/// the crate can use and which sources it compiles.
#[derive(Debug, Default, PartialEq)]
struct Manifest {
    /// The package it declares; none where it only holds a workspace.
    name: Option<String>,
    /// The packages it is built with, on any target.
    needs: Vec<Needed>,
    /// The packages only its tests and its build script are built with.
    extra: Vec<Needed>,
    /// It holds a workspace.
    workspace: bool,
    /// What that workspace calls a package, where that is not the package's
    /// own name.
    aliases: Vec<(String, String)>,
    /// The sources it names for its targets and its build script, as they
    /// are written: from the folder of the manifest.
    sources: Vec<String>,
    /// It is not the TOML this reader knows, as that of a newer Cargo may
    /// be, so what it names is not known.
    unread: bool,
}
/// The package a manifest declares, found line by line where the manifest
/// as a whole cannot be read.
fn declared(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .skip_while(|line| *line != "[package]")
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.trim() != "name" {
                return None;
            }
            let value = value.trim();
            let quote = value
                .chars()
                .next()
                .filter(|quote| ['"', '\''].contains(quote))?;
            let rest = &value[1..];
            Some(rest[..rest.find(quote)?].to_owned())
        })
}
/// Reads from a Cargo manifest what names packages and sources: the package
/// it declares, its tables of dependencies, those of one target among them,
/// and the paths of its targets.
fn manifest(text: &str) -> Manifest {
    // The names with an underscore are the ones Cargo read before.
    const EXTRA: [&str; 4] = [
        "dev-dependencies",
        "dev_dependencies",
        "build-dependencies",
        "build_dependencies",
    ];
    let Ok(table) = text.parse::<toml::Table>() else {
        return Manifest {
            name: declared(text),
            unread: true,
            ..Manifest::default()
        };
    };
    let listed = |table: &toml::Table, kinds: &[&str]| {
        kinds
            .iter()
            .filter_map(|kind| table.get(*kind)?.as_table())
            .flatten()
            .map(|(key, value)| Needed {
                key: key.clone(),
                package: value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
                workspace: value.get("workspace").and_then(toml::Value::as_bool) == Some(true),
            })
            .collect::<Vec<_>>()
    };
    let targets = table
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values().filter_map(toml::Value::as_table));
    let tables = std::iter::once(&table).chain(targets).collect::<Vec<_>>();
    // `[project]` is what `[package]` was called before.
    let package = table.get("package").or_else(|| table.get("project"));
    // A library is one table; programs, examples, tests and benchmarks are
    // lists of them.
    let mut sources = ["lib", "bin", "example", "test", "bench"]
        .iter()
        .filter_map(|kind| table.get(*kind))
        .flat_map(|built| match built {
            toml::Value::Array(several) => several.iter().collect::<Vec<_>>(),
            one => vec![one],
        })
        .filter_map(|target| Some(target.get("path")?.as_str()?.to_owned()))
        .collect::<Vec<_>>();
    sources.extend(
        package
            .and_then(|package| package.get("build")?.as_str())
            .map(str::to_owned),
    );
    Manifest {
        name: package
            .and_then(|package| package.get("name")?.as_str())
            .map(str::to_owned),
        needs: tables
            .iter()
            .flat_map(|table| listed(table, &["dependencies"]))
            .collect(),
        extra: tables
            .iter()
            .flat_map(|table| listed(table, &EXTRA))
            .collect(),
        workspace: table.contains_key("workspace"),
        aliases: table
            .get("workspace")
            .and_then(|workspace| workspace.get("dependencies")?.as_table())
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| {
                Some((key.clone(), value.get("package")?.as_str()?.to_owned()))
            })
            .collect(),
        sources,
        unread: false,
    }
}
/// What a Rust source says of other sources, as far as it tells which files
/// a crate compiles from outside its own folder.
#[derive(Debug, PartialEq)]
enum Said<'a> {
    /// A source named from the folder of the source that names it: by the
    /// `path` attribute of a module, or by an `include!` of a file.
    Beside(&'a str),
    /// A source an `include!` names from the folder of its package.
    Package(&'a str),
    /// A module that is a file of its own, or a folder of them.
    Module,
}
/// The pattern of what `said` finds.
static SAID: LazyLock<Option<regex::Regex>> = LazyLock::new(|| {
    regex::Regex::new(
        r##"(?x)
          \#!?\[ [^\]]*? \b path \s*=\s* "(?P<beside>[^"]*)"
        | \b include! \s* [(\[{] \s* "(?P<file>[^"]*)"
        | \b include! \s* [(\[{] \s* concat! \s* [(\[{] \s* env! \s* [(\[{] \s*
            "CARGO_MANIFEST_DIR" \s* [)\]}] \s* , \s* "(?P<package>[^"]*)"
        | \b mod \s+ (?:r\#)? [A-Za-z_][A-Za-z0-9_]* \s* ;
        "##,
    )
    .ok()
});
/// The places where a Rust source names another source, each with the text
/// that says so. This reads the text and is no parser. A path is taken
/// wherever it is written, in a comment too: one path too many only lets
/// more links stand. A module counts where it is declared in code, since
/// tests and documentation quote declarations, and a module that is only
/// quoted would bring a whole folder.
fn said(text: &str) -> Vec<(Said<'_>, &str)> {
    let Some(pattern) = SAID.as_ref() else {
        return Vec::new();
    };
    let found = pattern
        .captures_iter(text)
        .filter_map(|found| {
            let named = |name: &str| found.name(name).map(|written| written.as_str());
            let what = match (named("beside").or(named("file")), named("package")) {
                (Some(written), _) => Said::Beside(written),
                (None, Some(written)) => Said::Package(written),
                (None, None) => Said::Module,
            };
            Some((what, found.get(0)?))
        })
        .collect::<Vec<_>>();
    // The comments and strings up to the last module declared. Most sources
    // declare none, and are not read for theirs.
    let mut quiet = Vec::new();
    if let Some((_, last)) = found.iter().rfind(|(what, _)| *what == Said::Module) {
        let bytes = text.as_bytes();
        let mut at = 0;
        while at <= last.start() {
            match quiet_end(bytes, at) {
                Some(end) => {
                    quiet.push((at, end));
                    at = end;
                }
                None => at += 1,
            }
        }
    }
    found
        .into_iter()
        .filter(|(what, place)| {
            let after = quiet.partition_point(|(_, end)| *end <= place.start());
            *what != Said::Module
                || quiet
                    .get(after)
                    .is_none_or(|(begin, _)| place.start() < *begin)
        })
        .map(|(what, place)| (what, place.as_str()))
        .collect()
}
/// What is kept of a Rust source where the source itself is not: the text
/// of the places where it names other sources, which reads as the source
/// does. None for a source that names none.
pub fn modules(text: &str) -> Option<String> {
    let places = said(text)
        .into_iter()
        .map(|(_, place)| place)
        .collect::<Vec<_>>();
    (!places.is_empty()).then(|| places.join("\n"))
}
/// What is kept of the Rust source in a file, where it is one that is read.
pub fn kept(source: &Path) -> Option<String> {
    std::fs::symlink_metadata(source)
        .is_ok_and(|meta| meta.is_file() && meta.len() <= SOURCE_LIMIT)
        .then(|| std::fs::read_to_string(source).ok())
        .flatten()
        .and_then(|text| modules(&text))
}
/// Where a path written in a source or a manifest leads, from the folder it
/// is read from. None where it is absolute or leaves the tree.
fn led(folder: &str, written: &str) -> Option<String> {
    // A backslash in a literal is written twice.
    let written = written.replace("\\\\", "/").replace('\\', "/");
    let mut parts = folder
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if written.starts_with('/') || written.split('/').next()?.ends_with(':') {
        return None;
    }
    for part in written.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}
/// The Rust crates of a tree, and which of them each can use.
pub(crate) struct Crates {
    /// For each file of the graph, its crate: the package of the manifest
    /// that stands nearest above a Rust source; else `NONE`.
    of: Vec<u32>,
    /// The crates that compile a file besides its own: those that pull it
    /// in from outside their folder.
    more: HashMap<u32, Vec<u32>>,
    /// For each crate, the crates it can use: itself, what it or its tests
    /// and build script are built with, and what those are built with.
    /// None where that is not known, and the crate may use any.
    can: Vec<Option<HashSet<u32>>>,
}
impl Crates {
    /// Finds the crates of the files of a graph, with `text` giving a file
    /// of the tree by its path: a manifest, or a source.
    pub(crate) fn find(files: &[String], text: &dyn Fn(&str) -> Option<String>) -> Self {
        // The manifest of a folder, read once.
        let mut read: HashMap<&str, Option<Manifest>> = HashMap::new();
        let mut ids: HashMap<&str, u32> = HashMap::new();
        let mut folders: Vec<&str> = Vec::new();
        let mut of = vec![NONE; files.len()];
        for (file, path) in files.iter().enumerate() {
            if !path.ends_with(".rs") {
                continue;
            }
            // Up from the file: the first manifest met says whose the file
            // is, and those above it are read for what a workspace names.
            let mut folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
            let mut nearest = None;
            loop {
                let found = read.entry(folder).or_insert_with(|| {
                    let path = if folder.is_empty() {
                        "Cargo.toml".to_owned()
                    } else {
                        format!("{folder}/Cargo.toml")
                    };
                    text(&path).map(|text| manifest(&text))
                });
                if nearest.is_none()
                    && let Some(manifest) = found
                {
                    nearest = Some((folder, manifest.name.is_some()));
                }
                if folder.is_empty() {
                    break;
                }
                folder = folder.rsplit_once('/').map_or("", |(above, _)| above);
            }
            // A manifest that declares no package holds a workspace, and
            // what lies right under it is of no crate.
            if let Some((own, true)) = nearest {
                let next = folders.len() as u32;
                of[file] = *ids.entry(own).or_insert_with(|| {
                    folders.push(own);
                    next
                });
            }
        }
        let manifests = folders
            .iter()
            .map(|folder| read.get(folder).and_then(Option::as_ref))
            .collect::<Vec<_>>();
        // A package may stand in the tree twice, and two workspaces may
        // call different packages by one name: whatever a name may stand
        // for is taken, so that nothing true is ruled out.
        let mut named: HashMap<&str, Vec<u32>> = HashMap::new();
        for (at, manifest) in manifests.iter().enumerate() {
            if let Some(name) = manifest.and_then(|manifest| manifest.name.as_deref()) {
                named.entry(name).or_default().push(at as u32);
            }
        }
        let mut aliases: HashMap<&str, Vec<&str>> = HashMap::new();
        for manifest in read.values().flatten() {
            for (alias, package) in &manifest.aliases {
                aliases.entry(alias).or_default().push(package);
            }
        }
        // The crates of this tree a dependency may name.
        let local = |needed: &Needed| {
            let mut names = vec![needed.package.as_deref().unwrap_or(&needed.key)];
            if needed.workspace {
                names.push(&needed.key);
                names.extend(aliases.get(needed.key.as_str()).into_iter().flatten());
            }
            names
                .into_iter()
                .filter_map(|name| named.get(name))
                .flatten()
                .copied()
                .collect::<Vec<_>>()
        };
        // A manifest that cannot be read and names no package may be any
        // crate's, so nothing is ruled out. One that names its package is
        // built with what is not known.
        let unread = read.values().flatten().any(|manifest| manifest.unread);
        let lost = read
            .values()
            .flatten()
            .any(|manifest| manifest.unread && manifest.name.is_none());
        // Whether the workspace of a folder is in sight: a manifest that
        // holds one, there or above. It is not where the project is a
        // folder inside a larger workspace.
        let rooted = |folder: &str| {
            let mut at = folder;
            loop {
                if read
                    .get(at)
                    .and_then(Option::as_ref)
                    .is_some_and(|manifest| manifest.workspace)
                {
                    return true;
                }
                if at.is_empty() {
                    return false;
                }
                at = at.rsplit_once('/').map_or("", |(above, _)| above);
            }
        };
        // What a crate leaves to its workspace to name is known where that
        // workspace is in sight and every manifest was read: an unread one
        // may be the workspace, or a part of it.
        let known = folders
            .iter()
            .zip(&manifests)
            .map(|(folder, manifest)| {
                manifest.is_some_and(|manifest| {
                    !manifest.unread
                        && (!manifest
                            .needs
                            .iter()
                            .chain(&manifest.extra)
                            .any(|needed| needed.workspace)
                            || (!unread && rooted(folder)))
                })
            })
            .collect::<Vec<_>>();
        let can = (0..folders.len())
            .map(|own| {
                let first = manifests[own].filter(|_| !lost && known[own])?;
                let mut can = HashSet::from([own as u32]);
                let mut next = first
                    .needs
                    .iter()
                    .chain(&first.extra)
                    .flat_map(local)
                    .collect::<Vec<_>>();
                // What a crate is built with brings what itself is built
                // with, but not what its own tests are.
                while let Some(other) = next.pop() {
                    if can.insert(other) {
                        let manifest =
                            manifests[other as usize].filter(|_| known[other as usize])?;
                        next.extend(manifest.needs.iter().flat_map(local));
                    }
                }
                Some(can)
            })
            .collect();
        // Sources are read only where a link could be ruled out at all.
        let more = if folders.len() > 1 {
            Self::pulled(files, &of, &folders, &manifests, text)
        } else {
            HashMap::new()
        };
        Self { of, more, can }
    }
    /// The files a crate compiles from outside its own folder, each with the
    /// crates that pull it in. A source is pulled in by the `path` of a
    /// module, by an `include!` or by the path of a target; with it come
    /// the sources it names in turn, and the modules it declares, which lie
    /// beside it.
    fn pulled(
        files: &[String],
        of: &[u32],
        folders: &[&str],
        manifests: &[Option<&Manifest>],
        text: &dyn Fn(&str) -> Option<String>,
    ) -> HashMap<u32, Vec<u32>> {
        // What a source says, from the source or from what was kept of it.
        let read = |path: &str| text(path).or_else(|| text(&format!("{path}.{MODULES}")));
        // Whether a path lies in the folder of a crate, down to where the
        // folder of another crate begins.
        let own = |at: u32, path: &str| {
            let holds = |folder: &str| {
                folder.is_empty()
                    || path
                        .strip_prefix(folder)
                        .is_some_and(|rest| rest.starts_with('/'))
            };
            let folder = folders[at as usize];
            holds(folder)
                && !folders
                    .iter()
                    .any(|other| other.len() > folder.len() && holds(other))
        };
        let ids = files
            .iter()
            .enumerate()
            .map(|(at, path)| (path.as_str(), at as u32))
            .collect::<HashMap<_, _>>();
        // Where the sources lead that each file of a crate names.
        let mut names: HashMap<u32, Vec<String>> = HashMap::new();
        for (file, path) in files.iter().enumerate() {
            if of[file] == NONE {
                continue;
            }
            let Some(source) = read(path) else {
                continue;
            };
            let here = path.rsplit_once('/').map_or("", |(folder, _)| folder);
            let named = said(&source)
                .into_iter()
                .filter_map(|(what, _)| match what {
                    Said::Beside(written) => led(here, written),
                    Said::Package(written) => led(
                        folders[of[file] as usize],
                        written.trim_start_matches(['/', '\\']),
                    ),
                    Said::Module => None,
                })
                .collect::<Vec<_>>();
            if !named.is_empty() {
                names.insert(file as u32, named);
            }
        }
        // A source to look at, and a crate that compiles it.
        let mut next = names
            .iter()
            .flat_map(|(file, named)| named.iter().map(|path| (path.clone(), of[*file as usize])))
            .collect::<Vec<_>>();
        for (at, manifest) in manifests.iter().enumerate() {
            for written in manifest.iter().flat_map(|manifest| &manifest.sources) {
                next.extend(led(folders[at], written).map(|path| (path, at as u32)));
            }
        }
        let mut more: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut declares: HashMap<String, bool> = HashMap::new();
        while let Some((path, by)) = next.pop() {
            if own(by, &path) {
                continue;
            }
            // The modules a source declares lie beside it, so where it
            // declares any, its folder is taken with it.
            let beside = path.rsplit_once('/').map_or("", |(folder, _)| folder);
            let wide = *declares.entry(path.clone()).or_insert_with(|| {
                read(&path).is_some_and(|source| {
                    said(&source).iter().any(|(what, _)| *what == Said::Module)
                })
            });
            let taken = if wide {
                files
                    .iter()
                    .enumerate()
                    .filter(|(_, file)| {
                        beside.is_empty()
                            || file
                                .strip_prefix(beside)
                                .is_some_and(|rest| rest.starts_with('/'))
                    })
                    .map(|(at, _)| at as u32)
                    .collect::<Vec<_>>()
            } else {
                ids.get(path.as_str()).copied().into_iter().collect()
            };
            for file in taken {
                if of[file as usize] == NONE || of[file as usize] == by {
                    continue;
                }
                let crates = more.entry(file).or_default();
                if !crates.contains(&by) {
                    crates.push(by);
                    // What that source names, the crate compiles as well.
                    next.extend(
                        names
                            .get(&file)
                            .into_iter()
                            .flatten()
                            .map(|path| (path.clone(), by)),
                    );
                }
            }
        }
        more
    }
    /// Whether a link from one file to another cannot be: both are Rust
    /// sources of crates, and no crate that compiles the first is built
    /// with a crate that compiles the second.
    pub(crate) fn rules_out(&self, from: u32, to: u32) -> bool {
        let (user, used) = (self.of[from as usize], self.of[to as usize]);
        let all = |file: u32, own: u32| {
            std::iter::once(own).chain(self.more.get(&file).into_iter().flatten().copied())
        };
        user != NONE
            && used != NONE
            && all(from, user).all(|user| {
                self.can[user as usize]
                    .as_ref()
                    .is_some_and(|can| all(to, used).all(|used| !can.contains(&used)))
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_manifest_names_its_package_and_what_it_is_built_with() {
        let need = |key: &str, package: Option<&str>, workspace: bool| Needed {
            key: key.to_owned(),
            package: package.map(str::to_owned),
            workspace,
        };
        // A table is read in the order of its keys or of its lines; either
        // way the same dependencies are found.
        let sorted = |mut needed: Vec<Needed>| {
            needed.sort_by(|a, b| a.key.cmp(&b.key));
            needed
        };
        let read = manifest(
            r#"
[package]
name = "shop-app"   # the shop
version = "0.1.0"
build = "../build/common.rs"

[lib]
path = "src/shop.rs"

[[bin]]
name = "shop"
path = "../shared/main.rs"

[[bin]]
name = "plain"

[[test]]
name = "it"
path = "tests/it.rs"

[dependencies]
serde = "1"
core = { package = "shop-core", path = "../core" }
kit.workspace = true
log = { workspace = true, optional = true }

[dependencies.mail]
package = "shop-mail"
path = "../mail"

[target.'cfg(unix)'.dependencies]
libc = "0.2"

[target."cfg(windows)".dev-dependencies]
winapi = "0.3"

[dev-dependencies]
fixtures = { path = "../fixtures" }

[build_dependencies]
cc = "1"

[workspace.dependencies]
kit = { package = "shop-kit", path = "crates/kit" }
log = "0.4"
"#,
        );
        assert_eq!(read.name.as_deref(), Some("shop-app"));
        assert_eq!(
            sorted(read.needs),
            [
                need("core", Some("shop-core"), false),
                need("kit", None, true),
                need("libc", None, false),
                need("log", None, true),
                need("mail", Some("shop-mail"), false),
                need("serde", None, false),
            ]
        );
        assert_eq!(
            sorted(read.extra),
            [
                need("cc", None, false),
                need("fixtures", None, false),
                need("winapi", None, false),
            ]
        );
        // Only a name that stands for another package is kept.
        assert_eq!(read.aliases, [("kit".to_owned(), "shop-kit".to_owned())]);
        // The sources of its targets and of its build script, as written.
        let mut sources = read.sources;
        sources.sort();
        assert_eq!(
            sources,
            [
                "../build/common.rs",
                "../shared/main.rs",
                "src/shop.rs",
                "tests/it.rs"
            ]
        );
        assert!(!read.unread);
        // What `[package]` was called before, and a manifest of a workspace.
        assert_eq!(
            manifest("[project]\nname = \"old\"\n").name.as_deref(),
            Some("old")
        );
        assert_eq!(
            manifest("[workspace]\nmembers = [\"a\"]\n"),
            Manifest {
                workspace: true,
                ..Manifest::default()
            }
        );
        assert!(read.workspace && !manifest("[package]\nname = \"a\"\n").workspace);
        // A newer Cargo reads a table over several lines; this reader does
        // not, and says so, with the package it still finds by its line.
        let newer = "[package]\nversion = \"1\"\nname = 'shop-app' # the shop\n\n[dependencies]\nserde = {\n    version = \"1\",\n}\n";
        assert_eq!(
            manifest(newer),
            Manifest {
                name: Some("shop-app".to_owned()),
                unread: true,
                ..Manifest::default()
            }
        );
        for nameless in [
            "[package\nname = \"x\"\n",
            "[package]\nname = x y\n",
            "[workspace]\nmembers = [\n[package]\n",
            "[lib]\nname = \"x\"\n= 1\n",
        ] {
            assert_eq!(
                manifest(nameless),
                Manifest {
                    unread: true,
                    ..Manifest::default()
                },
                "{nameless}"
            );
        }
    }
    /// The crates of some files, with the manifests and the sources of
    /// their tree that can be read.
    fn crates(files: &[&str], texts: &[(&str, &str)]) -> (Vec<String>, Crates) {
        let files = files
            .iter()
            .map(|file| (*file).to_owned())
            .collect::<Vec<_>>();
        let crates = Crates::find(&files, &|path| {
            texts
                .iter()
                .find(|(at, _)| *at == path)
                .map(|(_, text)| (*text).to_owned())
        });
        (files, crates)
    }
    /// Whether a link from one file to another is left standing.
    fn stands((files, crates): &(Vec<String>, Crates), from: &str, to: &str) -> bool {
        let at = |path: &str| files.iter().position(|file| file == path).unwrap() as u32;
        !crates.rules_out(at(from), at(to))
    }
    const TREE: [&str; 12] = [
        "a/src/lib.rs",
        "a/tests/it.rs",
        "a/README.md",
        "a/tools/x/src/main.rs",
        "b/src/lib.rs",
        "c/src/lib.rs",
        "d/src/lib.rs",
        "e/src/lib.rs",
        "f/build.rs",
        "vendor/c/src/lib.rs",
        "scripts/gen.rs",
        "web/cart.php",
    ];
    const WORKSPACE: (&str, &str) = (
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\"]\n[workspace.dependencies]\nkit = { package = \"d\", path = \"d\" }\n",
    );
    const A: (&str, &str) = (
        "a/Cargo.toml",
        "[package]\nname = \"a\"\n[dependencies]\nb = { path = \"../b\" }\n",
    );
    const B: (&str, &str) = (
        "b/Cargo.toml",
        "[package]\nname = \"b\"\n[dependencies]\nc = \"1\"\n[dev-dependencies]\nd = { path = \"../d\" }\n",
    );
    const C: (&str, &str) = ("c/Cargo.toml", "[package]\nname = \"c\"\n");
    const D: (&str, &str) = ("d/Cargo.toml", "[package]\nname = \"d\"\n");
    const E: (&str, &str) = (
        "e/Cargo.toml",
        "[package]\nname = \"e\"\n[dependencies]\nsee = { package = \"c\", version = \"1\" }\nkit.workspace = true\n",
    );
    const F: (&str, &str) = (
        "f/Cargo.toml",
        "[package]\nname = \"f\"\n[target.'cfg(unix)'.dependencies]\nc = \"1\"\n[build-dependencies]\nd = \"1\"\n",
    );
    const X: (&str, &str) = ("a/tools/x/Cargo.toml", "[package]\nname = \"x\"\n");
    const COPY: (&str, &str) = ("vendor/c/Cargo.toml", "[package]\nname = \"c\"\n");
    #[test]
    fn a_crate_can_use_what_its_manifest_names_and_what_that_brings() {
        let tree = crates(&TREE, &[WORKSPACE, A, B, C, D, E, F, X, COPY]);
        let kept = |from: &str, to: &str| stands(&tree, from, to);
        // Itself, what it names, and what that is built with.
        assert!(kept("a/src/lib.rs", "a/tests/it.rs"));
        assert!(kept("a/src/lib.rs", "b/src/lib.rs"));
        assert!(kept("a/tests/it.rs", "c/src/lib.rs"));
        // Not what uses it, and not what the tests of another are built
        // with; its own tests are built with what it names for them.
        assert!(!kept("b/src/lib.rs", "a/src/lib.rs"));
        assert!(!kept("c/src/lib.rs", "b/src/lib.rs"));
        assert!(!kept("a/src/lib.rs", "d/src/lib.rs"));
        assert!(kept("b/src/lib.rs", "d/src/lib.rs"));
        assert!(!kept("d/src/lib.rs", "c/src/lib.rs"));
        // A package under another name, and under the name its workspace
        // gives it.
        assert!(kept("e/src/lib.rs", "c/src/lib.rs"));
        assert!(kept("e/src/lib.rs", "d/src/lib.rs"));
        assert!(!kept("e/src/lib.rs", "b/src/lib.rs"));
        // What one target is built with, and what the build script is.
        assert!(kept("f/build.rs", "c/src/lib.rs"));
        assert!(kept("f/build.rs", "d/src/lib.rs"));
        assert!(!kept("f/build.rs", "a/src/lib.rs"));
        // A package that stands in the tree twice may be either.
        assert!(kept("a/src/lib.rs", "vendor/c/src/lib.rs"));
        assert!(kept("f/build.rs", "vendor/c/src/lib.rs"));
        assert!(!kept("vendor/c/src/lib.rs", "c/src/lib.rs"));
        // The nearest manifest says whose a source is.
        assert!(!kept("a/tools/x/src/main.rs", "a/src/lib.rs"));
        assert!(!kept("a/src/lib.rs", "a/tools/x/src/main.rs"));
        // What is of no crate is left alone: a source right under the
        // workspace, and whatever is not Rust.
        for free in ["scripts/gen.rs", "web/cart.php", "a/README.md"] {
            assert!(kept(free, "d/src/lib.rs"), "{free}");
            assert!(kept("d/src/lib.rs", free), "{free}");
        }
        // Where the project is a folder inside a larger workspace, what
        // that workspace calls a package is out of sight, and a crate that
        // leaves a name to it may use any. The others are read as before.
        let inside = crates(&TREE, &[A, B, C, D, E, F, X, COPY]);
        assert!(stands(&inside, "e/src/lib.rs", "b/src/lib.rs"));
        assert!(!stands(&inside, "c/src/lib.rs", "b/src/lib.rs"));
        assert!(!stands(&inside, "a/src/lib.rs", "d/src/lib.rs"));
        // Without manifests there are no crates, and nothing is ruled out.
        let bare = crates(&TREE, &[]);
        assert!(stands(&bare, "c/src/lib.rs", "b/src/lib.rs"));
        assert!(bare.1.can.is_empty());
    }
    #[test]
    fn a_manifest_that_cannot_be_read_rules_nothing_out_for_what_leans_on_it() {
        const NEWER: &str =
            "[package]\nname = \"b\"\n[dependencies]\nc = {\n    version = \"1\",\n}\n";
        let tree = crates(
            &TREE,
            &[WORKSPACE, A, ("b/Cargo.toml", NEWER), C, D, E, F, X, COPY],
        );
        let kept = |from: &str, to: &str| stands(&tree, from, to);
        // What `b` is built with is not known, nor what it brings to `a`.
        assert!(kept("b/src/lib.rs", "a/src/lib.rs"));
        assert!(kept("b/src/lib.rs", "e/src/lib.rs"));
        assert!(kept("a/src/lib.rs", "d/src/lib.rs"));
        // Nor what the unread manifest says as a workspace, so a crate
        // that leaves a name to its workspace may use any.
        assert!(kept("e/src/lib.rs", "b/src/lib.rs"));
        // The others are read as before.
        assert!(!kept("c/src/lib.rs", "b/src/lib.rs"));
        assert!(!kept("f/build.rs", "a/src/lib.rs"));
        assert!(kept("f/build.rs", "c/src/lib.rs"));
        // An unread manifest that names no package may be any crate's.
        let lost = crates(
            &TREE,
            &[("Cargo.toml", "[workspace\n"), A, B, C, D, E, F, X, COPY],
        );
        for (from, to) in [
            ("c/src/lib.rs", "b/src/lib.rs"),
            ("f/build.rs", "a/src/lib.rs"),
            ("d/src/lib.rs", "c/src/lib.rs"),
        ] {
            assert!(stands(&lost, from, to), "{from} -> {to}");
        }
    }
    #[test]
    fn a_source_says_which_other_sources_it_names() {
        let source = r##"//! A crate.
#[path = "../../core/src/fs/spelling.rs"]
mod spelling;
#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(
    not(windows),
    path = "unix.rs"
)]
mod system;
pub mod plain;
mod r#async;
mod inline {
    include!("generated.rs");
    include! { concat!(env!("CARGO_MANIFEST_DIR"), "/../shared/tables.rs") }
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
    const NOTES: &str = include_str!("../notes.txt");
}
#[derive(Debug)]
struct Kept { path: String }
// #[path = "in/a/comment.rs"]
/* mod commented; /* include!("nested.rs"); */ mod still_commented; */
fn path<'a>(first: &'a str) -> &'static str {
    let path = "not/an/attribute.rs";
    let _ = ('"', '\'', "mod in_a_string;", r#"include!("raw.rs");"#);
    "#[path = \"in/a/string.rs\"]"
}
mod last;
const RAW: &[u8] = br#"one " quote"#;
mod after_bytes;
"##;
        let found = |text: &str| {
            said(text)
                .into_iter()
                .map(|(what, _)| match what {
                    Said::Beside(written) => format!("beside {written}"),
                    Said::Package(written) => format!("package {written}"),
                    Said::Module => "module".to_owned(),
                })
                .collect::<Vec<_>>()
        };
        let expected = [
            "beside ../../core/src/fs/spelling.rs",
            "module",
            "beside windows.rs",
            "beside unix.rs",
            "module",
            "module",
            "module",
            "beside generated.rs",
            "package /../shared/tables.rs",
            // A path is taken from a comment and from a string as well; a
            // module is not.
            "beside in/a/comment.rs",
            "beside nested.rs",
            "beside raw.rs",
            "module",
            "module",
        ];
        assert_eq!(found(source), expected);
        // What is kept of the source reads as the source does, and is a
        // small part of it.
        let kept = modules(source).unwrap();
        assert_eq!(found(&kept), expected);
        assert!(kept.starts_with("#[path = \"../../core/src/fs/spelling.rs\"\nmod spelling;\n"));
        assert!(!kept.contains("Kept"));
        assert_eq!(modules("fn main() {}\n// no module here\n"), None);
    }
    #[test]
    fn a_written_path_leads_from_the_folder_it_is_read_from() {
        let at = |folder: &str, written: &str| led(folder, written);
        assert_eq!(at("a/src", "x.rs").as_deref(), Some("a/src/x.rs"));
        assert_eq!(
            at("a/src", "../../b/src/x.rs").as_deref(),
            Some("b/src/x.rs")
        );
        assert_eq!(
            at("a/src", "./sub//y.rs").as_deref(),
            Some("a/src/sub/y.rs")
        );
        assert_eq!(at("", "x.rs").as_deref(), Some("x.rs"));
        assert_eq!(at("a", "../x.rs").as_deref(), Some("x.rs"));
        // As Windows writes it in a literal.
        assert_eq!(at("a/src", "..\\\\b\\\\x.rs").as_deref(), Some("a/b/x.rs"));
        // Out of the tree, and from the root of a disk.
        assert_eq!(at("a", "../../x.rs"), None);
        assert_eq!(at("a/src", "/input/source.rs"), None);
        assert_eq!(at("a/src", "C:\\\\input\\\\source.rs"), None);
    }
    #[test]
    fn a_source_pulled_in_by_its_path_is_of_the_crate_that_pulls_it_in_too() {
        let files = [
            "qa/src/lib.rs",
            "qa/src/check.rs",
            "core/src/lib.rs",
            "core/src/error.rs",
            "core/src/fs/spelling.rs",
            "core/src/fs/atomic.rs",
            "base/src/lib.rs",
            "base/src/main.rs",
            "cli/build.rs",
            "tool/src/lib.rs",
            "tool/src/other.rs",
            "tool/src/identity/mod.rs",
            "tool/src/identity/hash.rs",
            "probe/src/lib.rs",
        ];
        // Only the core names a crate: it is built with the base.
        let manifests = [
            ("Cargo.toml", "[workspace]\nmembers = [\"*\"]\n"),
            ("qa/Cargo.toml", "[package]\nname = \"qa\"\n"),
            (
                "core/Cargo.toml",
                "[package]\nname = \"core\"\n[dependencies]\nbase = { path = \"../base\" }\n",
            ),
            ("base/Cargo.toml", "[package]\nname = \"base\"\n"),
            ("cli/Cargo.toml", "[package]\nname = \"cli\"\n"),
            ("tool/Cargo.toml", "[package]\nname = \"tool\"\n"),
            // A program of the probe is a source of the base.
            (
                "probe/Cargo.toml",
                "[package]\nname = \"probe\"\n[[bin]]\nname = \"probe\"\npath = \"../base/src/main.rs\"\n",
            ),
        ];
        let sources = [
            // One source of the core, which declares no module.
            (
                "qa/src/lib.rs",
                "#[path = \"../../core/src/fs/spelling.rs\"]\nmod spelling;\npub mod check;\n",
            ),
            ("core/src/lib.rs", "pub mod error;\npub mod fs;\n"),
            ("core/src/fs/spelling.rs", "pub fn fold() {}\n"),
            // A source of the tool that declares a module beside it and
            // takes one of the base in turn.
            (
                "cli/build.rs",
                "#[path = \"../tool/src/identity/mod.rs\"]\nmod identity;\nfn main() {}\n",
            ),
            ("tool/src/lib.rs", "pub mod identity;\npub mod other;\n"),
            (
                "tool/src/identity/mod.rs",
                "mod hash;\ninclude!(\"../../../base/src/lib.rs\");\n",
            ),
            ("base/src/lib.rs", "pub fn zero() {}\n"),
        ];
        let check = |tree: &(Vec<String>, Crates)| {
            let kept = |from: &str, to: &str| stands(tree, from, to);
            // The pulled source is of the crate that pulls it in, and that
            // source alone: it declares no module.
            assert!(kept("qa/src/check.rs", "core/src/fs/spelling.rs"));
            assert!(!kept("qa/src/check.rs", "core/src/fs/atomic.rs"));
            assert!(!kept("qa/src/check.rs", "core/src/error.rs"));
            // It may use what either of its crates can.
            assert!(kept("core/src/fs/spelling.rs", "qa/src/check.rs"));
            assert!(kept("core/src/fs/spelling.rs", "base/src/lib.rs"));
            assert!(!kept("core/src/error.rs", "qa/src/check.rs"));
            // A source that declares a module brings its folder.
            assert!(kept("cli/build.rs", "tool/src/identity/mod.rs"));
            assert!(kept("cli/build.rs", "tool/src/identity/hash.rs"));
            assert!(!kept("cli/build.rs", "tool/src/other.rs"));
            // What a pulled source names in turn comes with it.
            assert!(kept("tool/src/other.rs", "base/src/lib.rs"));
            assert!(kept("cli/build.rs", "base/src/lib.rs"));
            assert!(!kept("cli/build.rs", "base/src/main.rs"));
            assert!(!kept("tool/src/other.rs", "core/src/error.rs"));
            // The source of a target that lies in another crate.
            assert!(kept("base/src/main.rs", "probe/src/lib.rs"));
            assert!(kept("probe/src/lib.rs", "base/src/main.rs"));
            assert!(!kept("base/src/lib.rs", "probe/src/lib.rs"));
        };
        let texts = manifests
            .iter()
            .chain(&sources)
            .copied()
            .collect::<Vec<_>>();
        check(&crates(&files, &texts));
        // The same is found where the sources are gone and what was kept of
        // them stands beside where they stood.
        let kept = sources
            .iter()
            .filter_map(|(path, text)| Some((format!("{path}.{MODULES}"), modules(text)?)))
            .collect::<Vec<_>>();
        assert_eq!(kept.len(), 5);
        let texts = manifests
            .iter()
            .copied()
            .chain(
                kept.iter()
                    .map(|(path, text)| (path.as_str(), text.as_str())),
            )
            .collect::<Vec<_>>();
        check(&crates(&files, &texts));
        // Without the sources nothing is known to be pulled in.
        let bare = crates(&files, &manifests);
        assert!(!stands(&bare, "qa/src/check.rs", "core/src/fs/spelling.rs"));
        assert!(stands(&bare, "probe/src/lib.rs", "base/src/main.rs"));
    }
}
