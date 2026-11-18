use super::*;
use std::io::Read;

pub(crate) const FILE: &str = ".harness-architecture.json";
const LIMIT: usize = 64 * 1024;
const MAX_LAYERS: usize = 32;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rule {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) paths: Vec<String>,
    pub(super) allows: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rules {
    version: u32,
    pub(super) layers: Vec<Rule>,
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
        let rules: Self = serde_json::from_slice(bytes).map_err(|error| {
            format!(
                "Invalid {FILE} at line {}, column {}. Check the JSON fields and types.",
                error.line(),
                error.column()
            )
        })?;
        if rules.version != 1 || !(1..=MAX_LAYERS).contains(&rules.layers.len()) {
            return Err(format!(
                "{FILE} requires version 1 and between 1 and {MAX_LAYERS} layers."
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
        }
        Ok(rules)
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
        from != to && !self.layers[from].allows.contains(&self.layers[to].id)
    }
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
