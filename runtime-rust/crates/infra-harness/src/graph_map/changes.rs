use super::*;

#[derive(Debug, Default)]
pub(super) struct CachedMemberFile {
    pub(super) path: String,
    pub(super) stamp: (u64, u128),
    pub(super) source: Arc<String>,
    pub(super) ir: Option<Arc<AstFileExtraction>>,
    pub(super) declared: Arc<Vec<DeclaredMember>>,
}

#[derive(Debug, Default)]
pub(super) struct CachedMemberLink {
    pub(super) caller: String,
    pub(super) target: String,
    pub(super) caller_stamp: (u64, u128),
    pub(super) target_stamp: (u64, u128),
    pub(super) refs: Arc<Vec<DepMemberRef>>,
    pub(super) warnings: Arc<Vec<VisibilityWarning>>,
}

#[derive(Debug, Default)]
pub(crate) struct PersistentMemberStore {
    pub(super) files: Vec<CachedMemberFile>,
    pub(super) links: Vec<CachedMemberLink>,
}

const STAT_RACY_NS: u128 = 2_000_000_000;

pub(super) fn disk_file_stamp(root: &Path, rel_path: &str) -> Option<(u64, u128)> {
    if rel_path.split('/').any(|part| part == "..") {
        return None;
    }
    let meta = std::fs::metadata(root.join(rel_path)).ok()?;
    if !meta.is_file() || meta.len() > SOURCE_LIMIT {
        return None;
    }
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0u128, |d| d.as_nanos());
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map_or(0u128, |d| d.as_nanos());
    // Skip caching file metadata inside the filesystem timestamp race window.
    if mtime_ns == 0 || now_ns == 0 || now_ns.saturating_sub(mtime_ns) < STAT_RACY_NS {
        return None;
    }
    Some((meta.len(), mtime_ns))
}

impl Index {
    pub(super) fn known_files_set(&self) -> &BTreeSet<String> {
        self.known_files_set
            .get_or_init(|| self.files.iter().cloned().collect())
    }
}

pub(super) type PairMemberRefs = (Arc<Vec<DepMemberRef>>, Arc<Vec<VisibilityWarning>>);

pub(crate) struct FileMemberCache<'a> {
    pub(super) index: &'a Index,
    pub(super) stamps: HashMap<String, Option<(u64, u128)>>,
    pub(super) sources: HashMap<String, Option<Arc<String>>>,
    pub(super) irs: HashMap<String, Option<Arc<AstFileExtraction>>>,
    pub(super) members: HashMap<String, Arc<Vec<DeclaredMember>>>,
    pub(super) links: HashMap<(String, String), PairMemberRefs>,
}

impl<'a> FileMemberCache<'a> {
    pub(super) fn new(index: &'a Index) -> Self {
        Self {
            index,
            stamps: HashMap::new(),
            sources: HashMap::new(),
            irs: HashMap::new(),
            members: HashMap::new(),
            links: HashMap::new(),
        }
    }

    pub(super) fn stamp(&mut self, path: &str) -> Option<(u64, u128)> {
        if let Some(&s) = self.stamps.get(path) {
            return s;
        }
        let s = disk_file_stamp(&self.index.root, path);
        self.stamps.insert(path.to_owned(), s);
        s
    }

    pub(super) fn load_file_entry(&mut self, path: &str) {
        if self.sources.contains_key(path) {
            return;
        }
        let stamp_opt = self.stamp(path);
        if let Some(stamp) = stamp_opt
            && let Ok(mut store) = self.index.member_store.lock()
            && let Some(pos) = store
                .files
                .iter()
                .position(|e| e.path == path && e.stamp == stamp)
        {
            let hit = store.files.remove(pos);
            let src = hit.source.clone();
            let ir = hit.ir.clone();
            let decls = hit.declared.clone();
            store.files.push(hit);
            self.sources.insert(path.to_owned(), Some(src));
            self.irs.insert(path.to_owned(), ir);
            self.members.insert(path.to_owned(), decls);
            return;
        }
        let loaded = text_at(&self.index.root, path).map(Arc::new);
        let ir = loaded
            .as_deref()
            .and_then(|s| parse_file_ir(path, s).map(Arc::new));
        let decls = Arc::new(
            ir.as_ref()
                .map(|parsed| parsed.declared_members.clone())
                .unwrap_or_default(),
        );
        if let (Some(stamp), Some(src)) = (stamp_opt, &loaded)
            && let Ok(mut store) = self.index.member_store.lock()
        {
            store.files.retain(|e| e.path != path);
            if store.files.len() >= MAX_CACHED_MEMBER_FILES {
                store.files.remove(0);
            }
            store.files.push(CachedMemberFile {
                path: path.to_owned(),
                stamp,
                source: src.clone(),
                ir: ir.clone(),
                declared: decls.clone(),
            });
        }
        self.sources.insert(path.to_owned(), loaded);
        self.irs.insert(path.to_owned(), ir);
        self.members.insert(path.to_owned(), decls);
    }

    pub(super) fn source(&mut self, path: &str) -> Option<&str> {
        self.load_file_entry(path);
        self.sources
            .get(path)
            .and_then(|s| s.as_deref().map(String::as_str))
    }

    pub(super) fn declared(&mut self, path: &str) -> &[DeclaredMember] {
        self.load_file_entry(path);
        self.members.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub(super) fn symbol_match(&mut self, at: u32) -> Option<DeclaredMember> {
        let symbol = &self.index.symbols[at as usize];
        if matches!(symbol.kind, Kind::File | Kind::Note | Kind::Heading) {
            return None;
        }
        let file_path = self.index.files[symbol.file as usize].as_str();
        let label = self.index.label(at);
        let owner_raw = self.index.owner(at);
        let owner = owner_raw
            .as_deref()
            .and_then(|o| o.rsplit('.').next())
            .or(owner_raw.as_deref());
        let hint = match symbol.kind {
            Kind::Method => Some("method"),
            Kind::Function => Some("function"),
            _ => None,
        };
        let decls = self.declared(file_path);
        find_matching_declared_member(decls, label, hint, owner).cloned()
    }

    pub(super) fn link_analysis(
        &mut self,
        caller_path: &str,
        target_path: &str,
    ) -> (Vec<DepMemberRef>, Vec<VisibilityWarning>) {
        let key = (caller_path.to_owned(), target_path.to_owned());
        if let Some((refs, warns)) = self.links.get(&key) {
            return ((**refs).clone(), (**warns).clone());
        }
        let caller_stamp_opt = self.stamp(caller_path);
        let target_stamp_opt = self.stamp(target_path);
        if let (Some(c_stamp), Some(t_stamp)) = (caller_stamp_opt, target_stamp_opt)
            && let Ok(mut store) = self.index.member_store.lock()
            && let Some(pos) = store.links.iter().position(|e| {
                e.caller == caller_path
                    && e.target == target_path
                    && e.caller_stamp == c_stamp
                    && e.target_stamp == t_stamp
            })
        {
            let hit = store.links.remove(pos);
            let refs = hit.refs.clone();
            let warns = hit.warnings.clone();
            store.links.push(hit);
            self.links.insert(key, (refs.clone(), warns.clone()));
            return ((*refs).clone(), (*warns).clone());
        }

        self.load_file_entry(target_path);
        self.load_file_entry(caller_path);
        let target_members = self
            .members
            .get(target_path)
            .cloned()
            .unwrap_or_else(|| Arc::new(Vec::new()));
        let caller_src_opt = self.sources.get(caller_path).and_then(|s| s.clone());
        let Some(caller_src) = caller_src_opt else {
            let empty = (Arc::new(Vec::new()), Arc::new(Vec::new()));
            self.links.insert(key, empty);
            return (Vec::new(), Vec::new());
        };
        let caller_ir_opt = self.irs.get(caller_path).and_then(|ir| ir.clone());
        let known = self.index.known_files_set();
        let mut refs = if let Some(caller_ir) = caller_ir_opt.as_deref() {
            find_matching_member_references_from_ir(
                caller_path,
                &caller_src,
                caller_ir,
                target_path,
                &target_members,
                known,
                None,
                None,
                None,
            )
        } else {
            find_matching_member_references(
                caller_path,
                &caller_src,
                target_path,
                &target_members,
                known,
            )
        };
        let root = &self.index.root;
        let caller_extends = caller_extends_target_in(
            caller_path,
            Some(&caller_src),
            target_path,
            &target_members,
            known,
            &mut |p| text_at(root, p),
        );
        let mut warnings = Vec::new();
        for mref in &mut refs {
            let assess = enrich_member_ref_in(
                Some(root),
                Some(known),
                mref,
                &target_members,
                caller_path,
                target_path,
                caller_extends,
            );
            if let Some(w) = assess.warning {
                push_unique_link_warning(&mut warnings, w);
            }
        }
        let refs_arc = Arc::new(refs.clone());
        let warns_arc = Arc::new(warnings.clone());
        if let (Some(c_stamp), Some(t_stamp)) = (caller_stamp_opt, target_stamp_opt)
            && let Ok(mut store) = self.index.member_store.lock()
        {
            store
                .links
                .retain(|e| !(e.caller == caller_path && e.target == target_path));
            if store.links.len() >= MAX_CACHED_MEMBER_LINKS {
                store.links.remove(0);
            }
            store.links.push(CachedMemberLink {
                caller: caller_path.to_owned(),
                target: target_path.to_owned(),
                caller_stamp: c_stamp,
                target_stamp: t_stamp,
                refs: refs_arc.clone(),
                warnings: warns_arc.clone(),
            });
        }
        self.links.insert(key, (refs_arc, warns_arc));
        (refs, warnings)
    }
}

pub(super) fn push_unique_link_member_ref(out: &mut Vec<DepMemberRef>, candidate: DepMemberRef) {
    let c_norm = norm_member_name(&candidate.name);
    if !out.iter().any(|existing| {
        norm_member_name(&existing.name) == c_norm
            && existing.kind == candidate.kind
            && existing.parent == candidate.parent
            && existing.access == candidate.access
    }) {
        out.push(candidate);
    }
}

pub(super) fn push_unique_link_warning(
    out: &mut Vec<VisibilityWarning>,
    warning: VisibilityWarning,
) {
    let w_norm = norm_member_name(&warning.member);
    if !out.iter().any(|existing| {
        norm_member_name(&existing.member) == w_norm
            && existing.kind == warning.kind
            && existing.parent == warning.parent
            && existing.line == warning.line
            && existing.reason == warning.reason
    }) {
        out.push(warning);
    }
}

pub(super) fn insert_symbol_overlay(
    obj: &mut serde_json::Map<String, Value>,
    member: Option<&DeclaredMember>,
    access_risk: Option<&str>,
) {
    if let Some(tm) = member {
        obj.insert("member_kind".to_owned(), json!(tm.kind));
        obj.insert("modifiers".to_owned(), json!(tm.modifiers));
        if let Some(vis) = &tm.visibility {
            obj.insert("visibility".to_owned(), json!(vis));
        }
    }
    if let Some(risk) = access_risk {
        obj.insert("access_risk".to_owned(), json!(risk));
    }
}
/// The text of a file of the graph's tree: one that lies inside the tree
/// and is no larger than a source worth reading.
pub(super) fn text_at(root: &Path, path: &str) -> Option<String> {
    if path.split('/').any(|part| part == "..") {
        return None;
    }
    let file = root.join(path);
    std::fs::metadata(&file)
        .is_ok_and(|meta| meta.is_file() && meta.len() <= SOURCE_LIMIT)
        .then(|| std::fs::read_to_string(&file).ok())
        .flatten()
}
/// The links between files: of the links between symbols, those that cross
/// a file boundary, each pair of files once, with how its links were found
/// and of which kinds they are.
pub(super) fn pairs_of(symbols: &[Symbol], uses: &[Use]) -> Vec<(u32, u32, Pair)> {
    let mut pairs: HashMap<(u32, u32), Pair> = HashMap::new();
    for (from, to, kind, guessed) in uses {
        let from_sym = &symbols[*from as usize];
        let to_sym = &symbols[*to as usize];
        // Semantic hubs (routes, tables, events, contracts) bridge cross-file callers via
        // contract/blast lenses without masquerading as direct code dependencies onto `anchor_file`.
        if from_sym.is_semantic || to_sym.is_semantic {
            continue;
        }
        let (from, to) = (from_sym.file, to_sym.file);
        if from == to {
            continue;
        }
        let pair = pairs.entry((from, to)).or_default();
        let bump =
            |kinds: &mut Vec<(u16, u64)>| match kinds.iter_mut().find(|(known, _)| known == kind) {
                Some((_, count)) => *count += 1,
                None => kinds.push((*kind, 1)),
            };
        if *guessed {
            pair.guessed += 1;
        } else {
            pair.read += 1;
            bump(&mut pair.read_kinds);
        }
        bump(&mut pair.kinds);
    }
    let mut pairs = pairs
        .into_iter()
        .map(|((from, to), pair)| (from, to, pair))
        .collect::<Vec<_>>();
    pairs.sort_by_key(|(from, to, _)| (*from, *to));
    pairs
}
/// Whether a file is a test by where it lies or how it is named across supported languages.
pub(super) fn is_test(path: &str) -> bool {
    crate::graph::file_types::is_test_path(path)
}
/// The attributes a line of Rust begins with, and what follows them on the
/// line: whether one of them marks test code, and the rest of the line.
/// `#[test]`, a test macro of a runtime or a crate (`#[tokio::test]`,
/// `#[rstest]`) and `#[cfg(test)]` with its combinations mark test code;
/// `#[cfg(not(test))]` and a feature that is merely named `test` do not.
pub(super) fn attributes(line: &str) -> (bool, &str) {
    let mut rest = line.trim_start();
    let mut marked = false;
    while let Some(inner) = rest.strip_prefix("#[") {
        // The attribute ends at the bracket that closes it; brackets inside
        // a string do not count.
        let (mut depth, mut quoted, mut end) = (1usize, false, None);
        for (at, letter) in inner.char_indices() {
            match letter {
                '"' => quoted = !quoted,
                '[' if !quoted => depth += 1,
                ']' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(at);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            // It goes on over the next lines.
            return (marked || marks_test(inner), "");
        };
        marked |= marks_test(&inner[..end]);
        rest = inner[end + 1..].trim_start();
    }
    (marked, rest)
}
/// Whether the inside of a Rust attribute marks what follows as a test.
pub(super) fn marks_test(inner: &str) -> bool {
    let named = inner
        .find(|letter: char| !(letter.is_ascii_alphanumeric() || letter == '_' || letter == ':'))
        .unwrap_or(inner.len());
    let (name, told) = inner.split_at(named);
    match name.rsplit("::").next().unwrap_or(name) {
        "test" | "rstest" | "test_case" | "bench" | "wasm_bindgen_test" => true,
        "cfg" => {
            // What a string says is a name of a feature, not the word.
            let mut quoted = false;
            let plain = told
                .chars()
                .filter(|letter| {
                    if *letter == '"' {
                        quoted = !quoted;
                    }
                    !quoted && *letter != '"' && *letter != ' '
                })
                .collect::<String>();
            !plain.contains("not(test")
                && plain
                    .split(|letter: char| !(letter.is_ascii_alphanumeric() || letter == '_'))
                    .any(|word| word == "test")
        }
        _ => false,
    }
}
/// Where a comment, a string or a character that begins at byte `at` ends:
/// the byte after its last. None where none of them begins there.
pub(crate) fn quiet_end(bytes: &[u8], at: usize) -> Option<usize> {
    let next = bytes.get(at + 1).copied();
    // Whether what stands before a byte is no part of a name.
    let free =
        |at: usize| at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
    match bytes[at] {
        b'/' if next == Some(b'/') => Some(
            bytes[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |line| at + line),
        ),
        b'/' if next == Some(b'*') => {
            // Block comments nest.
            let mut depth = 1usize;
            let mut end = at + 2;
            while end < bytes.len() && depth > 0 {
                match (bytes[end], bytes.get(end + 1)) {
                    (b'/', Some(b'*')) => {
                        depth += 1;
                        end += 2;
                    }
                    (b'*', Some(b'/')) => {
                        depth -= 1;
                        end += 2;
                    }
                    _ => end += 1,
                }
            }
            Some(end)
        }
        b'"' => {
            let mut end = at + 1;
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            Some(end + 1)
        }
        // A raw string: `r"…"`, `r#"…"#`, also of bytes and for C, `br"…"`
        // and `cr"…"`. It is no part of a name, and `r#name` is a name.
        b'r' if free(at) || (matches!(bytes[at - 1], b'b' | b'c') && free(at - 1)) => {
            let hashes = bytes[at + 1..]
                .iter()
                .take_while(|byte| **byte == b'#')
                .count();
            if bytes.get(at + 1 + hashes) != Some(&b'"') {
                return None;
            }
            let mut close = at + 2 + hashes;
            while close < bytes.len()
                && !(bytes[close] == b'"'
                    && bytes[close + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|byte| **byte == b'#')
                        .count()
                        == hashes)
            {
                close += 1;
            }
            Some(close + hashes + 1)
        }
        b'\'' if next == Some(b'\\') => {
            // An escaped character: to the quote that closes it.
            let mut end = at + 3;
            while end < bytes.len() && bytes[end] != b'\'' {
                end += 1;
            }
            Some(end + 1)
        }
        b'\'' => {
            // One character between quotes; else a lifetime.
            let wide = match next {
                Some(byte) if byte >= 0xf0 => 4,
                Some(byte) if byte >= 0xe0 => 3,
                Some(byte) if byte >= 0xc0 => 2,
                _ => 1,
            };
            (bytes.get(at + 1 + wide) == Some(&b'\'')).then_some(at + 2 + wide)
        }
        _ => None,
    }
}
/// Where the Rust item that begins at byte `from` ends: at the `;` that
/// ends it, or at the brace that closes its body. What stands in a string,
/// a character or a comment does not count: a test often holds a source of
/// its own. The last byte of the text where the item is never closed.
pub(super) fn item_end(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let (mut braces, mut groups) = (0usize, 0usize);
    let mut at = from;
    while at < bytes.len() {
        if let Some(end) = quiet_end(bytes, at) {
            at = end;
            continue;
        }
        match bytes[at] {
            b'{' => braces += 1,
            b'}' => {
                braces = braces.saturating_sub(1);
                if braces == 0 {
                    return at;
                }
            }
            b'(' | b'[' => groups += 1,
            b')' | b']' => groups = groups.saturating_sub(1),
            b';' if braces == 0 && groups == 0 => return at,
            _ => {}
        }
        at += 1;
    }
    bytes.len().saturating_sub(1)
}
/// The runs of lines of a Rust source that are tests: an item marked as
/// one, most often `#[cfg(test)] mod tests { … }` or a `#[test]` function,
/// from its attribute to where it ends. Lines count from 1.
pub(super) fn rust_tests(text: &str) -> Vec<(u32, u32)> {
    let rows = lines(text);
    // Where each line begins, to tell the line of a place in the text.
    let mut starts = Vec::with_capacity(rows.len());
    let mut begun = 0;
    for line in text.split_inclusive('\n') {
        starts.push(begun);
        begun += line.len();
    }
    let raw = text.lines().map(str::trim).collect::<Vec<_>>();
    let mut spans = Vec::new();
    // The line of the attribute that marked what comes next.
    let mut marked: Option<usize> = None;
    let mut at = 0;
    while at < raw.len() {
        let (marks, rest) = attributes(raw[at]);
        if marks && marked.is_none() {
            marked = Some(at);
        }
        // The item itself: what follows the attributes of a line, or a
        // line of code. Comments and other attributes may stand between.
        let item = if raw[at].starts_with("#[") {
            rest
        } else if rows[at].kind == Row::Code {
            raw[at]
        } else {
            if rows[at].kind == Row::Closer {
                marked = None;
            }
            ""
        };
        let Some(first) = marked.filter(|_| !item.is_empty()) else {
            if !item.is_empty() {
                marked = None;
            }
            at += 1;
            continue;
        };
        marked = None;
        // The item is a part of the text, so it says where it stands in it.
        let from = item.as_ptr() as usize - text.as_ptr() as usize;
        let close = item_end(text, from);
        let end = starts
            .partition_point(|start| *start <= close)
            .saturating_sub(1)
            .max(at);
        spans.push((first as u32 + 1, end as u32 + 1));
        at = end + 1;
    }
    spans
}

/// Keep only the line ranges needed for Without tests in a revision graph.
pub(crate) fn kept_tests(source: &Path) -> std::io::Result<Vec<(u32, u32)>> {
    if std::fs::symlink_metadata(source)?.len() > SOURCE_LIMIT {
        return Ok(Vec::new());
    }
    match std::fs::read_to_string(source) {
        Ok(source) => Ok(rust_tests(&source)),
        // The ordinary source reader also skips files that are not UTF-8.
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}
pub(crate) type TestSpans = BTreeMap<String, Vec<(u32, u32)>>;
pub(crate) const TEST_SPANS: &str = "test-spans.json";

impl Index {
    /// The same graph without its tests: without the files that are tests
    /// by where they lie or how they are named, and, in a Rust source that
    /// can be read, without the symbols of its test modules and test
    /// functions. The links of what is left are counted anew, so a link
    /// that only a test made is gone from its file too. It is made once and
    /// kept with the graph.
    pub fn without_tests(&self) -> Arc<Index> {
        self.plain
            .get_or_init(|| Arc::new(self.plain_with(&|path| text_at(&self.root, path))))
            .clone()
    }
    /// What a graph without tests leaves out: how many files that are
    /// tests, and how many tests inside the files it keeps.
    pub fn left_out(&self) -> (u64, u64) {
        self.left
    }
    pub(super) fn test_symbols(
        &self,
        gone: &[bool],
        source: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<bool> {
        // A symbol goes with its file, with the test it is, or with the
        // test that holds it.
        let mut out = vec![false; self.symbols.len()];
        for (file, path) in self.files.iter().enumerate() {
            let own = self.first[file] as usize..self.first[file + 1] as usize;
            if gone[file] {
                out[own].fill(true);
                continue;
            }
            let persisted_in_ir = self.backend.as_deref() == Some(crate::graph_ir::NATIVE_IR_TOOL)
                || own.clone().any(|at| self.symbols[at].is_test);
            if persisted_in_ir {
                for at in own {
                    out[at] = self.symbols[at].is_test;
                }
                continue;
            }
            if !path.ends_with(".rs") || own.is_empty() {
                continue;
            }
            let scanned;
            let spans = if let Some(tests) = &self.revision_tests {
                tests.get(path).map(Vec::as_slice).unwrap_or_default()
            } else {
                scanned = source(path)
                    .map(|text| rust_tests(&text))
                    .unwrap_or_default();
                &scanned
            };
            for at in own {
                let line = self.symbols[at].line;
                out[at] = line > 0 && spans.iter().any(|(from, to)| (*from..=*to).contains(&line));
            }
        }
        for at in 0..self.symbols.len() {
            let mut holder = self.symbols[at].parent;
            // A graph may hold two symbols in each other; the walk is short.
            for _ in 0..32 {
                if holder == NONE || out[at] {
                    break;
                }
                out[at] = out[holder as usize];
                holder = self.symbols[holder as usize].parent;
            }
        }
        out
    }

    /// The graph without its tests, with `source` giving the text of a file
    /// where it can be read.
    pub(super) fn plain_with(&self, source: &dyn Fn(&str) -> Option<String>) -> Index {
        let gone = self
            .files
            .iter()
            .map(|path| is_test(path))
            .collect::<Vec<_>>();
        let out = self.test_symbols(&gone, source);
        let mut files = Vec::new();
        let mut kept_to_old = Vec::new();
        let mut file_at = vec![NONE; self.files.len()];
        for (file, path) in self.files.iter().enumerate() {
            if !gone[file] {
                file_at[file] = files.len() as u32;
                kept_to_old.push(file as u32);
                files.push(path.clone());
            }
        }
        let mut symbols = Vec::new();
        let mut symbol_at = vec![NONE; self.symbols.len()];
        let mut first = vec![0u32; files.len() + 1];
        let mut inside = 0u64;
        for (at, symbol) in self.symbols.iter().enumerate() {
            if out[at] {
                inside += u64::from(!gone[symbol.file as usize]);
                continue;
            }
            symbol_at[at] = symbols.len() as u32;
            first[file_at[symbol.file as usize] as usize + 1] += 1;
            symbols.push(*symbol);
        }
        for file in 0..files.len() {
            first[file + 1] += first[file];
        }
        for symbol in &mut symbols {
            symbol.file = file_at[symbol.file as usize];
            if symbol.parent != NONE {
                symbol.parent = symbol_at[symbol.parent as usize];
            }
        }
        // Of the links, those between what is left, the ones ruled out as
        // the ones that stand.
        let left = |links: &[Use]| {
            links
                .iter()
                .filter(|(from, to, ..)| !out[*from as usize] && !out[*to as usize])
                .map(|(from, to, kind, guessed)| {
                    (
                        symbol_at[*from as usize],
                        symbol_at[*to as usize],
                        *kind,
                        *guessed,
                    )
                })
                .collect::<Vec<Use>>()
        };
        let (uses, ruled) = (left(&self.uses), left(&self.ruled));
        let ties = left(&self.ties);
        let unlinked = (self.uses.len() - uses.len()) + (self.ruled.len() - ruled.len());
        let dropped = self.symbols.len() - symbols.len();
        let crates = self.crates.remap(&kept_to_old, self.files.len());
        Index {
            index_id: super::next_index_id(),
            root: self.root.clone(),
            revision_tests: None,
            test_symbols: OnceLock::new(),
            plain: OnceLock::new(),
            wires: OnceLock::new(),
            patterns: Default::default(),
            access: Default::default(),
            circles: Default::default(),
            ties,
            left: (gone.iter().filter(|gone| **gone).count() as u64, inside),
            ruled,
            crates,
            files,
            pairs: pairs_of(&symbols, &uses),
            symbols,
            first,
            names: self.names.clone(),
            ir_node_ids: String::new(),
            ir_node_id_spans: Vec::new(),
            community_names: self.community_names.clone(),
            nodes: self.nodes.saturating_sub(dropped as u64),
            links: self.links.saturating_sub(unlinked as u64),
            uses,
            kinds: self.kinds.clone(),
            unplaced: self.unplaced,
            per_file_struct_links: Vec::new(),
            per_file_edge_extra_links: Vec::new(),
            cross_extra_links: Vec::new(),
            cross_unplaced_links: Vec::new(),
            orphan_extra_links: 0,
            backend: self.backend.clone(),
            ir_db_path: self.ir_db_path.clone(),
            known_files_set: OnceLock::new(),
            manifest_dirs: OnceLock::new(),
            member_store: Mutex::new(PersistentMemberStore::default()),
        }
    }
}
/// A place of the change map: one changed file, a folder of them, or a file
/// that is linked to the change.
#[derive(Default)]
pub(super) struct Spot {
    pub(super) files: u64,
    pub(super) changed: u64,
    pub(super) status: Option<char>,
    pub(super) folder: bool,
    /// What a file holds: classes, methods, functions.
    pub(super) symbols: u64,
    pub(super) out: u64,
    pub(super) into: u64,
    /// A file outside the change that uses it, and so may be affected.
    pub(super) affected: bool,
    /// A file outside the change that the change uses.
    pub(super) needed: bool,
    /// Changed as well, outside the open folder.
    pub(super) altered: bool,
}
impl Spot {
    pub(super) fn role(&self) -> &'static str {
        match (self.changed > 0, self.affected, self.needed) {
            (true, ..) => "changed",
            (_, true, true) => "both",
            (_, true, false) => "affected",
            _ => "needed",
        }
    }
}
/// The shortest run of trailing path parts that no other drawn place ends
/// with: `Cart.php`, or `Order/Cart.php` where two files share the name.
pub(super) fn label<'a>(path: &'a str, all: &[&str]) -> &'a str {
    let mut start = path.rfind('/').map_or(0, |at| at + 1);
    loop {
        let tail = &path[start..];
        let shared = all.iter().any(|other| {
            *other != path
                && other
                    .strip_suffix(tail)
                    .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
        });
        if !shared || start == 0 {
            return tail;
        }
        start = path[..start - 1].rfind('/').map_or(0, |at| at + 1);
    }
}
impl Index {
    /// The dependency map of a change: the changed files, the files they use
    /// and the files that use them. `changed` names each file with Git's
    /// letter for what happened to it. `scope` narrows the change to one
    /// folder, `filter` keeps the linked files whose path contains it, and
    /// `focus` lists both sides of one place.
    ///
    /// Up to `SINGLE` changed files are drawn one by one. A larger change is
    /// drawn as the children of the open folder that hold changed files, and
    /// a folder is opened to reach its files. Linked files are always files,
    /// strongest first. A scope that names a file opens that file, changed
    /// or not.
    pub fn changes(
        &self,
        project: &str,
        changed: &[(String, char)],
        scope: &str,
        focus: Option<&str>,
        certain: bool,
        filter: &str,
    ) -> Value {
        if let Some(view) = self.symbols(project, scope, changed, None, focus, certain, filter) {
            return view;
        }
        let ids = self
            .files
            .iter()
            .enumerate()
            .map(|(id, file)| (file.as_str(), id as u32))
            .collect::<HashMap<_, _>>();
        let within = |scope: &str, path: &str| {
            scope.is_empty()
                || path
                    .strip_prefix(scope)
                    .is_some_and(|rest| rest.starts_with('/'))
        };
        // The changed files the graph knows, with what happened to each.
        let (mut removed, mut unknown) = (0u64, 0u64);
        let mut known = Vec::new();
        for (path, kind) in changed {
            if *kind == 'D' {
                removed += 1;
                continue;
            }
            match ids.get(path.as_str()) {
                Some(id) => known.push((*id, *kind)),
                None => unknown += 1,
            }
        }
        let here = |scope: &str| {
            known
                .iter()
                .map(|(id, _)| self.files[*id as usize].as_str())
                .filter(|path| within(scope, path))
                .collect::<Vec<_>>()
        };
        let mut scope = scope.trim_matches('/');
        if here(scope).is_empty() {
            scope = "";
        }
        // A folder whose changes all lie in one folder below it says no
        // more than that folder does, so it is passed through.
        loop {
            let inside = here(scope);
            let mut below = None;
            let narrow = inside.len() > SINGLE
                && inside.iter().all(|path| {
                    let (child, _) = group(scope, path);
                    child.len() < path.len() && *below.get_or_insert(child) == child
                });
            match below {
                Some(child) if narrow => scope = child,
                _ => break,
            }
        }
        // Inside the open folder is the change drawn here; what changed
        // outside it is told apart.
        let mut status: HashMap<u32, char> = HashMap::new();
        let mut elsewhere = std::collections::HashSet::new();
        for (id, kind) in &known {
            if within(scope, &self.files[*id as usize]) {
                status.insert(*id, *kind);
            } else {
                elsewhere.insert(*id);
            }
        }
        let single = status.len() <= SINGLE;
        let key = |id: u32| {
            let path = self.files[id as usize].as_str();
            if single { path } else { group(scope, path).0 }
        };
        let mut spots: BTreeMap<&str, Spot> = BTreeMap::new();
        for (id, kind) in &status {
            let place = key(*id);
            let spot = spots.entry(place).or_default();
            spot.files += 1;
            spot.changed += 1;
            spot.symbols += self.holds(*id);
            spot.folder |= place.len() < self.files[*id as usize].len();
            // A folder shows how many changed, even when it is one.
            spot.status = (!spot.folder && spot.changed == 1).then_some(*kind);
        }
        let mut flows: HashMap<(&str, &str), EdgeFlow> = HashMap::new();
        // Each linked file: how strongly, and on which side of the change.
        let mut near: HashMap<u32, (u64, bool, bool)> = HashMap::new();
        for (from, to, pair) in &self.pairs {
            let weight = pair.weight(certain);
            let (ours, theirs) = (status.contains_key(from), status.contains_key(to));
            if weight == 0 || !(ours || theirs) {
                continue;
            }
            let source = if ours {
                key(*from)
            } else {
                self.files[*from as usize].as_str()
            };
            let target = if theirs {
                key(*to)
            } else {
                self.files[*to as usize].as_str()
            };
            if source == target {
                continue;
            }
            if !ours {
                let entry = near.entry(*from).or_default();
                entry.0 += weight;
                entry.1 = true;
            }
            if !theirs {
                let entry = near.entry(*to).or_default();
                entry.0 += weight;
                entry.2 = true;
            }
            flows
                .entry((source, target))
                .or_default()
                .add_pair(pair, certain);
        }
        for ((source, target), edge) in &flows {
            if let Some(spot) = spots.get_mut(source) {
                spot.out += edge.flow.links;
            }
            if let Some(spot) = spots.get_mut(target) {
                spot.into += edge.flow.links;
            }
        }
        // A change too wide to draw keeps its busiest places.
        let places = spots.len();
        if places > SHOWN - AROUND {
            let mut order = spots
                .iter()
                .map(|(place, spot)| (spot.changed, spot.out + spot.into, *place))
                .collect::<Vec<_>>();
            order.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
            for (.., place) in order.into_iter().skip(SHOWN - AROUND) {
                spots.remove(place);
            }
        }
        let hidden = places - spots.len();
        let (affected, needed) = near.values().fold((0u64, 0u64), |(users, used), near| {
            (users + u64::from(near.1), used + u64::from(near.2))
        });
        let needle = filter.trim().to_lowercase();
        let linked = near.len();
        let mut around = near
            .iter()
            .map(|(id, near)| (self.files[*id as usize].as_str(), *id, *near))
            .filter(|(path, ..)| needle.is_empty() || path.to_lowercase().contains(&needle))
            .collect::<Vec<_>>();
        let matching = around.len();
        around.sort_by(|a, b| b.2.0.cmp(&a.2.0).then(a.0.cmp(b.0)));
        around.truncate(SHOWN.saturating_sub(spots.len()).max(AROUND));
        for (path, id, (_, uses, used)) in &around {
            spots.insert(
                path,
                Spot {
                    files: 1,
                    symbols: self.holds(*id),
                    affected: *uses,
                    needed: *used,
                    altered: elsewhere.contains(id),
                    ..Spot::default()
                },
            );
        }
        for ((source, target), edge) in &flows {
            if let Some(spot) = spots.get_mut(source)
                && spot.changed == 0
            {
                spot.out += edge.flow.links;
            }
            if let Some(spot) = spots.get_mut(target)
                && spot.changed == 0
            {
                spot.into += edge.flow.links;
            }
        }
        let mut edges = flows
            .iter()
            .filter(|((source, target), _)| {
                spots.contains_key(source) && spots.contains_key(target)
            })
            .collect::<Vec<_>>();
        let between = edges.len();
        edges.sort_by(|((a, b), left), ((c, d), right)| {
            right
                .flow
                .links
                .cmp(&left.flow.links)
                .then((a, b).cmp(&(c, d)))
        });
        edges.truncate(DRAWN);
        let drawn = spots.keys().copied().collect::<Vec<_>>();
        let named = |place: &str, spot: &Spot| {
            if spot.folder {
                name(place).to_owned()
            } else {
                label(place, &drawn).to_owned()
            }
        };
        let mut nodes = spots.iter().collect::<Vec<_>>();
        nodes.sort_by(|(left, a), (right, b)| {
            (b.changed > 0)
                .cmp(&(a.changed > 0))
                .then((b.out + b.into).cmp(&(a.out + a.into)))
                .then(left.cmp(right))
        });
        let focus = focus.and_then(|place| spots.get_key_value(place));
        let focus = focus.map(|(place, spot)| {
            // The files this place stands for, and whether a link's other
            // end counts: any file for a changed place, the change itself
            // for a file beside it.
            let own = |id: &u32| {
                if spot.changed > 0 {
                    status.contains_key(id) && key(*id) == *place
                } else {
                    self.files[*id as usize] == **place
                }
            };
            let counts = |id: &u32| spot.changed > 0 || status.contains_key(id);
            let mut uses: HashMap<u32, Flow> = HashMap::new();
            let mut used: HashMap<u32, Flow> = HashMap::new();
            let mut kinds: BTreeMap<u16, u64> = BTreeMap::new();
            for (from, to, pair) in &self.pairs {
                let weight = pair.weight(certain);
                if weight == 0 {
                    continue;
                }
                let (other, side) = match (own(from), own(to)) {
                    (true, false) => (to, &mut uses),
                    (false, true) => (from, &mut used),
                    _ => continue,
                };
                if !counts(other) {
                    continue;
                }
                let flow = side.entry(*other).or_default();
                flow.links += weight;
                flow.guessed += if certain { 0 } else { pair.guessed };
                if own(from) {
                    add_kind_counts(&mut kinds, pair.kinds_for(certain));
                }
            }
            let side = |rows: HashMap<u32, Flow>| {
                let mut rows = rows
                    .into_iter()
                    .map(|(id, flow)| (self.files[id as usize].as_str(), id, flow))
                    .collect::<Vec<_>>();
                rows.sort_by(|a, b| b.2.links.cmp(&a.2.links).then(a.0.cmp(b.0)));
                let more = rows.len().saturating_sub(LISTED);
                rows.truncate(LISTED);
                let rows = rows
                    .into_iter()
                    .map(|(path, id, flow)| {
                        json!({
                            "path": path,
                            "name": path,
                            "folder": false,
                            "inside": spots.contains_key(path),
                            "links": flow.links,
                            "guessed": flow.guessed,
                            "changed": status.contains_key(&id) || elsewhere.contains(&id),
                        })
                    })
                    .collect::<Vec<_>>();
                (rows, more)
            };
            let (uses, uses_more) = side(uses);
            let (used_by, used_by_more) = side(used);
            json!({
                "path": place,
                "name": named(place, spot),
                "folder": spot.folder,
                "inside": true,
                "files": spot.files,
                "symbols": spot.symbols,
                "within": 0,
                "out": spot.out,
                "in": spot.into,
                "changed": spot.changed,
                "status": spot.status.map(String::from),
                "role": spot.role(),
                "altered": spot.altered,
                "uses": uses,
                "uses_more": uses_more,
                "used_by": used_by,
                "used_by_more": used_by_more,
                "kinds": self.kinds_json(&kinds),
            })
        });
        json!({
            "mode": "changes",
            "scope": scope,
            "crumbs": Self::crumbs(project, scope),
            "certain": certain,
            "totals": self.totals(),
            "change": {
                "files": changed.len(),
                "graphed": status.len() + elsewhere.len(),
                "here": status.len(),
                "removed": removed,
                "unknown": unknown,
                "single": single,
                "hidden": hidden,
                "affected": affected,
                "needed": needed,
            },
            "children": linked,
            "matching": matching,
            "nodes": nodes.iter().map(|(place, spot)| json!({
                "path": place,
                "name": named(place, spot),
                "folder": spot.folder,
                "files": spot.files,
                "symbols": spot.symbols,
                "within": 0,
                "out": spot.out,
                "in": spot.into,
                "changed": spot.changed,
                "status": spot.status.map(String::from),
                "role": spot.role(),
                "altered": spot.altered,
            })).collect::<Vec<_>>(),
            "between": between,
            "backbone": false,
            "edges": edges.iter().map(|((source, target), edge)| {
                self.edge_json(source, target, edge)
            }).collect::<Vec<_>>(),
            "focus": focus,
        })
    }
}
