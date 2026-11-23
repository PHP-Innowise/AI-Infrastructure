use super::*;
use std::io::Read;

pub(crate) const FILE: &str = ".harness-architecture.json";
const LIMIT: usize = 64 * 1024;
const MAX_LAYERS: usize = 32;

pub(super) struct Rule {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) paths: Vec<String>,
    pub(super) allows: Vec<String>,
    pub(super) explicit_allows: bool,
    pub(super) forbids: Vec<String>,
    pub(super) forbids_semantic: Vec<String>,
    pub(super) max_cycles: Option<usize>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    name: String,
    paths: Vec<String>,
    #[serde(default)]
    allows: Option<Vec<String>>,
    #[serde(default)]
    forbids: Vec<String>,
    #[serde(default)]
    forbids_semantic: Vec<String>,
    #[serde(default)]
    max_cycles: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackageRule {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
    #[serde(default)]
    pub(crate) allows: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) forbids: Vec<String>,
}

fn default_semantic_effect() -> String {
    "forbid".to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SemanticRule {
    #[serde(default)]
    pub(crate) id: Option<String>,
    pub(crate) from: String,
    pub(crate) relation: String,
    #[serde(default)]
    pub(crate) target: Option<String>,
    #[serde(default = "default_semantic_effect")]
    pub(crate) effect: String,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

pub(crate) struct Rules {
    version: u32,
    pub(super) layers: Vec<Rule>,
    pub(super) package_rules: Vec<PackageRule>,
    pub(super) enforce_crate_boundaries: bool,
    pub(super) max_cycles: Option<usize>,
    pub(super) semantic_rules: Vec<SemanticRule>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRules {
    version: u32,
    #[serde(default)]
    layers: Vec<RawRule>,
    #[serde(default)]
    package_rules: Vec<PackageRule>,
    #[serde(default)]
    enforce_crate_boundaries: bool,
    #[serde(default)]
    max_cycles: Option<usize>,
    #[serde(default)]
    semantic_rules: Vec<SemanticRule>,
}

impl Rules {
    pub(crate) fn read(root: &Path) -> std::result::Result<Option<Self>, String> {
        let path = root.join(FILE);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(meta) if meta.is_file() && meta.len() <= LIMIT as u64 => {}
            _ => return Err(format!("{FILE} must be a regular file of at most 64 KiB.")),
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(LIMIT as u64 + 1).read_to_end(&mut bytes))
            .map_err(|_| format!("Could not read {FILE}."))?;
        Self::parse(&bytes).map(Some)
    }

    pub(crate) fn parse(bytes: &[u8]) -> std::result::Result<Self, String> {
        if bytes.len() > LIMIT {
            return Err(format!("{FILE} exceeds 64 KiB."));
        }
        let raw: RawRules = serde_json::from_slice(bytes).map_err(|error| {
            format!(
                "Invalid {FILE} at line {}, column {}. Check the JSON fields and types.",
                error.line(),
                error.column()
            )
        })?;
        let layers: Vec<Rule> = raw
            .layers
            .into_iter()
            .map(|r| {
                let explicit_allows = r.allows.is_some();
                Rule {
                    id: r.id,
                    name: r.name,
                    paths: r.paths,
                    allows: r.allows.unwrap_or_default(),
                    explicit_allows,
                    forbids: r.forbids,
                    forbids_semantic: r.forbids_semantic,
                    max_cycles: r.max_cycles,
                }
            })
            .collect();
        let rules = Self {
            version: raw.version,
            layers,
            package_rules: raw.package_rules,
            enforce_crate_boundaries: raw.enforce_crate_boundaries,
            max_cycles: raw.max_cycles,
            semantic_rules: raw.semantic_rules,
        };
        if rules.version != 1
            || rules.layers.len() > MAX_LAYERS
            || (rules.layers.is_empty() && !rules.has_extended_rules())
        {
            return Err(format!(
                "{FILE} requires version 1 and between 1 and {MAX_LAYERS} layers (or extended package/cycle/semantic rules)."
            ));
        }
        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for layer in &rules.layers {
            if layer.id.is_empty()
                || layer.id.len() > 64
                || !layer
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || !ids.insert(layer.id.as_str())
            {
                return Err("Layer IDs must be unique and use at most 64 letters, digits, hyphens or underscores.".into());
            }
            if layer.name.trim().is_empty()
                || layer.name.len() > 160
                || layer.name.chars().any(char::is_control)
                || layer.paths.is_empty()
            {
                return Err(format!(
                    "Layer {} needs a name and at least one path.",
                    layer.id
                ));
            }
            for path in &layer.paths {
                if !is_valid_pattern(path) || !paths.insert(path.as_str()) {
                    return Err(format!(
                        "Layer {} needs unique project-relative paths or safe glob patterns.",
                        layer.id
                    ));
                }
            }
        }
        for layer in &rules.layers {
            if layer.allows.iter().any(|id| !ids.contains(id.as_str())) {
                return Err(format!("Layer {} allows an unknown layer ID.", layer.id));
            }
            if layer.forbids.iter().any(|id| !ids.contains(id.as_str())) {
                return Err(format!("Layer {} forbids an unknown layer ID.", layer.id));
            }
            if layer.forbids_semantic.iter().any(|s| s.trim().is_empty()) {
                return Err(format!(
                    "Layer {} has an empty forbids_semantic entry.",
                    layer.id
                ));
            }
        }
        let mut pkg_ids = HashSet::new();
        for pkg in &rules.package_rules {
            if pkg.id.trim().is_empty() || pkg.id.len() > 96 || !pkg_ids.insert(pkg.id.as_str()) {
                return Err("Package rule IDs must be non-empty and unique.".into());
            }
            for path in &pkg.paths {
                if !is_valid_pattern(path) {
                    return Err(format!("Package rule {} has an invalid path glob.", pkg.id));
                }
            }
            if pkg.forbids.iter().any(|p| p.trim().is_empty())
                || pkg
                    .allows
                    .as_ref()
                    .is_some_and(|list| list.iter().any(|p| p.trim().is_empty()))
            {
                return Err(format!(
                    "Package rule {} allows/forbids entries must not be empty.",
                    pkg.id
                ));
            }
        }
        for sem in &rules.semantic_rules {
            if sem.from.trim().is_empty() || sem.relation.trim().is_empty() {
                return Err("Semantic rules require non-empty `from` and `relation`.".into());
            }
            if !ids.contains(sem.from.as_str()) && sem.from != "*" && !is_valid_pattern(&sem.from) {
                return Err(format!(
                    "Semantic rule `from` ({}) must name a declared layer or valid glob.",
                    sem.from
                ));
            }
            if let Some(ref target) = sem.target
                && target.trim().is_empty()
            {
                return Err("Semantic rule `target` cannot be blank.".into());
            }
            if !matches!(sem.effect.as_str(), "forbid" | "allow") {
                return Err(format!(
                    "Semantic rule `effect` must be \"forbid\" or \"allow\" (got {}).",
                    sem.effect
                ));
            }
        }
        Ok(rules)
    }

    pub(crate) fn has_extended_rules(&self) -> bool {
        !self.package_rules.is_empty()
            || self.enforce_crate_boundaries
            || self.max_cycles.is_some()
            || !self.semantic_rules.is_empty()
            || self.layers.iter().any(|l| {
                !l.forbids.is_empty() || !l.forbids_semantic.is_empty() || l.max_cycles.is_some()
            })
    }

    /// Deterministic SHA-256 fingerprint over architecture version and all layer/package/cycle/semantic definitions.
    pub(crate) fn sha256(&self) -> [u8; 32] {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(self.version.to_le_bytes());
        h.update((self.layers.len() as u64).to_le_bytes());
        for layer in &self.layers {
            h.update(layer.id.as_bytes());
            h.update([0]);
            h.update(layer.name.as_bytes());
            h.update([0]);
            h.update((layer.paths.len() as u64).to_le_bytes());
            for path in &layer.paths {
                h.update(path.as_bytes());
                h.update([0]);
            }
            h.update([u8::from(layer.explicit_allows)]);
            h.update((layer.allows.len() as u64).to_le_bytes());
            for allow in &layer.allows {
                h.update(allow.as_bytes());
                h.update([0]);
            }
            h.update((layer.forbids.len() as u64).to_le_bytes());
            for forbid in &layer.forbids {
                h.update(forbid.as_bytes());
                h.update([0]);
            }
            h.update((layer.forbids_semantic.len() as u64).to_le_bytes());
            for sem in &layer.forbids_semantic {
                h.update(sem.as_bytes());
                h.update([0]);
            }
            h.update(
                layer
                    .max_cycles
                    .map_or(u64::MAX, |v| v as u64)
                    .to_le_bytes(),
            );
        }
        h.update([u8::from(self.enforce_crate_boundaries)]);
        h.update(self.max_cycles.map_or(u64::MAX, |v| v as u64).to_le_bytes());
        h.update((self.package_rules.len() as u64).to_le_bytes());
        for pkg in &self.package_rules {
            h.update(pkg.id.as_bytes());
            h.update([0]);
            h.update(pkg.name.as_deref().unwrap_or("").as_bytes());
            h.update([0]);
            for p in &pkg.paths {
                h.update(p.as_bytes());
                h.update([0]);
            }
            if let Some(ref allows) = pkg.allows {
                h.update([1]);
                for a in allows {
                    h.update(a.as_bytes());
                    h.update([0]);
                }
            } else {
                h.update([0]);
            }
            for f in &pkg.forbids {
                h.update(f.as_bytes());
                h.update([0]);
            }
        }
        h.update((self.semantic_rules.len() as u64).to_le_bytes());
        for sem in &self.semantic_rules {
            h.update(sem.id.as_deref().unwrap_or("").as_bytes());
            h.update([0]);
            h.update(sem.from.as_bytes());
            h.update([0]);
            h.update(sem.relation.as_bytes());
            h.update([0]);
            h.update(sem.target.as_deref().unwrap_or("*").as_bytes());
            h.update([0]);
            h.update(sem.effect.as_bytes());
            h.update([0]);
        }
        h.finalize().into()
    }

    pub(super) fn layer(&self, path: &str) -> Option<usize> {
        let clean = path.trim_matches('/');
        if clean.is_empty() || clean.split('/').any(|part| matches!(part, "" | "." | "..")) {
            return None;
        }
        self.layers
            .iter()
            .enumerate()
            .flat_map(|(id, layer)| {
                layer.paths.iter().filter_map(move |pattern| {
                    match_specificity(pattern, clean).map(|spec| (id, spec))
                })
            })
            .max_by(|(id_a, spec_a), (id_b, spec_b)| {
                spec_a.cmp(spec_b).then_with(|| id_b.cmp(id_a))
            })
            .map(|(id, _)| id)
    }

    pub(super) fn forbidden(&self, from: usize, to: usize) -> bool {
        if from == to {
            return false;
        }
        let src = &self.layers[from];
        let dst_id = &self.layers[to].id;
        if src.forbids.iter().any(|id| id == dst_id) {
            return true;
        }
        if src.explicit_allows {
            return !src.allows.iter().any(|id| id == dst_id);
        }
        false
    }

    /// Resolves the package/crate identifier for `path` (from `package_rules` or manifest crate bucket).
    pub(super) fn package_id_for_path(
        &self,
        path: &str,
        crate_bucket: &str,
    ) -> Option<(Option<usize>, String)> {
        let clean = path.trim_matches('/');
        if !self.package_rules.is_empty() {
            if let Some((idx, _)) = self
                .package_rules
                .iter()
                .enumerate()
                .flat_map(|(idx, rule)| {
                    rule.paths.iter().filter_map(move |pat| {
                        match_specificity(pat, clean).map(|spec| (idx, spec))
                    })
                })
                .max_by(|(ia, sa), (ib, sb)| sa.cmp(sb).then_with(|| ib.cmp(ia)))
            {
                return Some((Some(idx), self.package_rules[idx].id.clone()));
            }
            if !crate_bucket.is_empty()
                && let Some((idx, rule)) = self.package_rules.iter().enumerate().find(|(_, r)| {
                    r.id == crate_bucket
                        || crate_bucket
                            .rsplit('/')
                            .next()
                            .is_some_and(|leaf| leaf == r.id)
                })
            {
                return Some((Some(idx), rule.id.clone()));
            }
        }
        if self.enforce_crate_boundaries && !crate_bucket.is_empty() && crate_bucket != "." {
            return Some((None, crate_bucket.to_owned()));
        }
        None
    }

    /// Evaluates package / crate boundary rules between two files.
    pub(super) fn check_package_violation(
        &self,
        from_path: &str,
        to_path: &str,
        from_bucket: &str,
        to_bucket: &str,
    ) -> Option<(Option<String>, String)> {
        let (from_rule_idx, from_pkg) = self.package_id_for_path(from_path, from_bucket)?;
        let (_, to_pkg) = self.package_id_for_path(to_path, to_bucket)?;
        if from_pkg == to_pkg {
            return None;
        }
        if let Some(idx) = from_rule_idx {
            let rule = &self.package_rules[idx];
            if rule.forbids.iter().any(|f| f == "*" || f == &to_pkg) {
                return Some((
                    Some(rule.id.clone()),
                    format!("Package `{from_pkg}` forbids dependencies on `{to_pkg}`."),
                ));
            }
            if let Some(ref allows) = rule.allows
                && !allows.iter().any(|a| a == "*" || a == &to_pkg)
            {
                return Some((
                    Some(rule.id.clone()),
                    format!("Package `{from_pkg}` does not allow dependencies on `{to_pkg}`."),
                ));
            }
        }
        if self.enforce_crate_boundaries && is_internal_package_path(to_path) {
            return Some((
                None,
                format!(
                    "Package `{from_pkg}` reaches into internal module of `{to_pkg}` ({to_path})."
                ),
            ));
        }
        None
    }

    /// Evaluates layer `forbids_semantic` and top-level `semantic_rules` on a semantic edge.
    pub(super) fn check_semantic_violation(
        &self,
        from_path: &str,
        from_layer: Option<usize>,
        relation: &str,
        target_label: &str,
    ) -> Option<(Option<String>, String)> {
        let rel_lower = relation.trim().to_ascii_lowercase();
        let target_clean = target_label.trim().trim_start_matches("sem:");
        let target_bare = target_clean
            .split_once(':')
            .map_or(target_clean, |(_, rest)| rest.trim());

        let layer_id = from_layer.map(|idx| self.layers[idx].id.as_str());
        let matches_sem_target = |pat: &str| -> bool {
            let p = pat.trim();
            p == "*"
                || p.eq_ignore_ascii_case(target_clean)
                || p.eq_ignore_ascii_case(target_bare)
                || match_glob(p, target_clean)
                || match_glob(p, target_bare)
        };

        // Check explicit `semantic_rules` (`allow` overrides `forbid` if later/equal, otherwise last matching rule wins).
        let mut matched_rule: Option<&SemanticRule> = None;
        for rule in &self.semantic_rules {
            let from_matches = rule.from == "*"
                || layer_id.is_some_and(|lid| lid == rule.from)
                || match_glob(&rule.from, from_path.trim_matches('/'));
            if !from_matches {
                continue;
            }
            let rel_matches =
                rule.relation == "*" || rule.relation.eq_ignore_ascii_case(&rel_lower);
            if !rel_matches {
                continue;
            }
            let target_matches = rule.target.as_deref().is_none_or(&matches_sem_target);
            if target_matches {
                matched_rule = Some(rule);
            }
        }
        if let Some(rule) = matched_rule {
            if rule.effect == "allow" {
                return None;
            }
            let msg = rule.message.clone().unwrap_or_else(|| {
                format!(
                    "Forbidden semantic edge `{}` -> `{}` ({}) from `{from_path}`.",
                    rule.from, rel_lower, target_clean
                )
            });
            return Some((rule.id.clone(), msg));
        }

        if let Some(idx) = from_layer {
            let layer = &self.layers[idx];
            for entry in &layer.forbids_semantic {
                let entry_trimmed = entry.trim();
                let matches_entry =
                    if let Some((rule_rel, rule_pat)) = entry_trimmed.split_once(':') {
                        (rule_rel == "*" || rule_rel.eq_ignore_ascii_case(&rel_lower))
                            && matches_sem_target(rule_pat)
                    } else {
                        entry_trimmed == "*" || entry_trimmed.eq_ignore_ascii_case(&rel_lower)
                    };
                if matches_entry {
                    return Some((
                        Some(layer.id.clone()),
                        format!(
                            "Layer `{}` forbids semantic edge `{rel_lower}` -> `{target_clean}`.",
                            layer.name
                        ),
                    ));
                }
            }
        }
        None
    }
}

fn is_internal_package_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower
        .split('/')
        .any(|seg| matches!(seg, "internal" | "private" | "_internal"))
        || lower.ends_with("_internal.rs")
        || lower.ends_with("_internal.ts")
        || lower.ends_with("_internal.py")
}

fn is_valid_pattern(pattern: &str) -> bool {
    if pattern.is_empty()
        || pattern.len() > 1024
        || pattern.contains(['\\', ':'])
        || pattern.chars().any(char::is_control)
    {
        return false;
    }
    for part in pattern.split('/') {
        if matches!(part, "" | "." | "..") {
            return false;
        }
        if part.contains("**") && part != "**" {
            // Restrict `**` to full path segments so ambiguous intra-segment `**` forms are refused.
            return false;
        }
    }
    true
}

fn match_segment(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = text.chars().collect();
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star_pi, mut star_si) = (None, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = Some(pi);
            star_si = si;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn match_glob(pattern: &str, path: &str) -> bool {
    let mut p_segs = Vec::new();
    for seg in pattern.split('/') {
        if seg == "**" && p_segs.last().copied() == Some("**") {
            continue;
        }
        p_segs.push(seg);
    }
    if p_segs.last().copied() == Some("**") {
        // A trailing `/**` matches files beneath that directory (at least one path segment).
        p_segs.push("*");
    }
    let s_segs: Vec<&str> = path.split('/').collect();
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star_pi, mut star_si) = (None, 0usize);
    while si < s_segs.len() {
        if pi < p_segs.len() && p_segs[pi] == "**" {
            star_pi = Some(pi);
            star_si = si;
            pi += 1;
        } else if pi < p_segs.len() && match_segment(p_segs[pi], s_segs[si]) {
            pi += 1;
            si += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }
    while pi < p_segs.len() && p_segs[pi] == "**" {
        pi += 1;
    }
    pi == p_segs.len()
}

fn match_specificity(pattern: &str, path: &str) -> Option<(bool, usize, bool, bool, usize)> {
    let has_glob = pattern.contains(['*', '?']);
    if !has_glob {
        let exact = path == pattern;
        let matches = exact
            || path
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with('/'));
        if !matches {
            return None;
        }
        let literal = pattern.bytes().filter(|b| *b != b'/').count();
        return Some((exact, literal, true, true, pattern.len()));
    }
    if !match_glob(pattern, path) {
        return None;
    }
    let has_double_star = pattern.split('/').any(|seg| seg == "**");
    let literal = pattern
        .bytes()
        .filter(|b| !matches!(*b, b'/' | b'*' | b'?'))
        .count();
    Some((false, literal, !has_double_star, false, pattern.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_validate_paths_and_resolve_more_specific_assignments() {
        let input = json!({"version":1, "layers":[
            {"id":"outer","name":"Outer","paths":["src"],"allows":["inner"]},
            {"id":"inner","name":"Inner","paths":["src/area","shared.php"],"allows":[]}
        ]});
        let parse = |value: &Value| Rules::parse(value.to_string().as_bytes());
        let rules = parse(&input).unwrap();
        assert_eq!(rules.layer("src/area/Port.php"), Some(1));
        assert_eq!(rules.layer("src/areal/Port.php"), Some(0));
        assert_eq!(rules.layer("shared.php"), Some(1));
        assert_eq!(rules.layer("shared.php.bak"), None);
        assert!(rules.forbidden(1, 0));
        assert!(!rules.forbidden(0, 1));
        assert!(!rules.forbidden(1, 1));
        let mut wrong = input.clone();
        wrong["layers"][1]["paths"] = json!(["src"]);
        assert!(parse(&wrong).is_err());
        wrong["layers"][1]["paths"] = json!(["../outside"]);
        assert!(parse(&wrong).is_err());
        wrong["layers"][1]["paths"] = json!(["other"]);
        wrong["layers"][1]["allows"] = json!(["missing"]);
        assert!(parse(&wrong).is_err());
    }

    #[test]
    fn rules_support_safe_globs_and_reject_unsafe_patterns() {
        let parse = |value: &Value| Rules::parse(value.to_string().as_bytes());
        let input = json!({"version":1, "layers":[
            {"id":"domain","name":"Domain","paths":["src/**","**/Domain/*.php"],"allows":[]},
            {"id":"controllers","name":"Controllers","paths":["src/**/*Controller.php","src/V?/Api.php"],"allows":["domain"]},
            {"id":"direct","name":"Direct","paths":["src/*/Helper.php"],"allows":["domain"]},
            {"id":"health","name":"Health","paths":["src/HealthController.php"],"allows":[]}
        ]});
        let rules = parse(&input).unwrap();

        // `**` matches 0+ segments, while trailing `/**` matches files under `src/`.
        assert_eq!(rules.layer("src/Order.php"), Some(0));
        assert_eq!(rules.layer("src/Billing/Entity/Invoice.php"), Some(0));
        assert_eq!(rules.layer("modules/Domain/User.php"), Some(0));
        assert_eq!(rules.layer("src"), None);

        // `*` matches within one segment only; `?` matches one character.
        assert_eq!(rules.layer("src/Orders/Helper.php"), Some(2));
        assert_eq!(rules.layer("src/Orders/Sub/Helper.php"), Some(0));
        assert_eq!(rules.layer("src/V1/Api.php"), Some(1));
        assert_eq!(rules.layer("src/V10/Api.php"), Some(0));

        // Specificity: exact path > more specific glob > broader glob.
        assert_eq!(rules.layer("src/OrderController.php"), Some(1));
        assert_eq!(rules.layer("src/Nested/OrderController.php"), Some(1));
        assert_eq!(rules.layer("src/HealthController.php"), Some(3));

        // Unsafe patterns must be rejected.
        for bad in [
            "/src/*.php",
            "../src/*.php",
            "src/../secret/*.php",
            "src/**/../secret.php",
            "./src/*.php",
            "src/",
            "src//a.php",
            "src\\*.php",
            "C:src/*.php",
            "src/**foo.php",
            "",
        ] {
            let mut invalid = input.clone();
            invalid["layers"][0]["paths"] = json!([bad]);
            assert!(parse(&invalid).is_err(), "pattern `{bad}` must be rejected");
        }
    }
}
