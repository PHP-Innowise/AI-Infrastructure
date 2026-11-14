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
                if path.len() > 1024
                    || path.contains(['\\', ':', '*', '?'])
                    || path.chars().any(char::is_control)
                    || path.split('/').any(|part| matches!(part, "" | "." | ".."))
                    || !paths.insert(path.as_str())
                {
                    return Err(format!(
                        "Layer {} needs unique project-relative paths without wildcards.",
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
        self.layers
            .iter()
            .enumerate()
            .flat_map(|(id, layer)| layer.paths.iter().map(move |prefix| (id, prefix.as_str())))
            .filter(|(_, prefix)| {
                path == *prefix
                    || path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|(_, prefix)| prefix.len())
            .map(|(id, _)| id)
    }

    pub(super) fn forbidden(&self, from: usize, to: usize) -> bool {
        from != to && !self.layers[from].allows.contains(&self.layers[to].id)
    }
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
}
