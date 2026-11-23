//! The SkillCache capsule `z = (μ, B, P, π, V)`: its JSON shape and strict reader.
//!
//! A capsule is what a compiler agent returns for one observed use of a skill:
//! what it is for (`mu`), the values known before control is handed over
//! (`bindings`), the programs a local executor runs (`programs`), the ordered
//! plan (`steps`) and the checks that follow it (`verification`). Observed
//! values (`examples`) are a separate document and never part of a capsule.
//!
//! The reader refuses what it does not know: an unknown field, a missing one,
//! a value over its limit. Every refusal is a [`Violation`] with a stable code,
//! so a repair round can be told exactly what to change.
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The only capsule shape this engine reads and writes.
pub const SCHEMA_VERSION: u64 = 1;
pub const CAPSULE_BYTES: usize = 256 * 1024;
pub const EXAMPLES_BYTES: usize = 1024 * 1024;
/// A bound value is a scalar, a path or a short list: never file content.
pub const BINDING_BYTES: usize = 4 * 1024;
pub const TEXT_CHARS: usize = 4000;
pub const DESCRIPTION_CHARS: usize = 600;
pub const PROGRAMS: usize = 32;
/// `steps` and `verification` together.
pub const PLAN_STEPS: usize = 64;
pub const BINDINGS: usize = 32;
pub const NAMES: usize = 32;
pub const ARGUMENTS: usize = 16;
pub const ARGUMENT_BYTES: usize = 512;
pub const TIMEOUT_SECONDS: std::ops::RangeInclusive<u64> = 1..=1800;
pub const SOURCE_SKILLS: usize = 8;
pub const RUNTIMES: usize = 8;

/// One reason a capsule, its examples or its package was refused.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Violation {
    /// Stable identifier of the rule, such as `dangling_input`.
    pub code: &'static str,
    /// Where it applies, such as `programs[1].outputs[0]`.
    pub at: String,
    pub message: String,
}
impl Violation {
    pub fn new(code: &'static str, at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            at: at.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Form {
    /// Programs with residual instructions; an executor runs the plan.
    Hybrid,
    /// A shorter written procedure (`capsule.md`); nothing is executed.
    Textual,
}
impl Form {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::Textual => "textual",
        }
    }
}

/// Origin class of a SkillCache capsule.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CapsuleKind {
    #[default]
    Skill,
    /// Verified read-only/verification sequence synthesized from tool chains without an explicit skill.
    ToolChain,
    /// Portable edition template candidate awaiting target-repo Gate 2 + Gate 3 verification.
    Template,
}

impl CapsuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::ToolChain => "tool_chain",
            Self::Template => "template",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "skill" => Some(Self::Skill),
            "tool_chain" => Some(Self::ToolChain),
            "template" => Some(Self::Template),
            _ => None,
        }
    }
}

/// What a capsule is selected by and shown as.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mu {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSkill {
    pub name: String,
    /// Project-relative path of the skill's entry document.
    pub path: String,
    /// Digest of every regular file of the skill when the capsule was made.
    pub tree_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Network {
    None,
    Required,
}
impl Network {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Required => "required",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub network: Network,
    /// Bare names of the interpreters the programs start, such as `python3`.
    pub runtimes: Vec<String>,
    /// Optional safe folder/file names re-enabled for change tracking when ignored by default.
    pub allow_ignored: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingType {
    String,
    Path,
    Number,
    Boolean,
    List,
}
impl BindingType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Path => "path",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::List => "list",
        }
    }
    /// Whether an observed or supplied value has this type.
    pub fn admits(self, value: &Value) -> bool {
        match self {
            Self::String | Self::Path => value.is_string(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::List => value.as_array().is_some_and(|items| {
                items
                    .iter()
                    .all(|item| item.is_string() || item.is_number() || item.is_boolean())
            }),
        }
    }
}

/// A value known before control is handed to the agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub name: String,
    pub kind: BindingType,
    pub origin: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    pub id: String,
    pub entry: Vec<String>,
    pub inputs: Vec<String>,
    /// Only the values later programs read. Answers go in the result.
    pub outputs: Vec<String>,
    pub check: Vec<String>,
    pub timeout_seconds: u64,
    pub read_only: bool,
    pub description: Option<String>,
}

/// Where in the source skill a residual instruction comes from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRef {
    pub path: String,
    pub section: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step {
    /// A residual step of the plan the agent performs itself.
    Instruction {
        text: String,
        source: Option<SourceRef>,
        produces: Vec<String>,
    },
    Program {
        program: String,
    },
    /// A verification step the agent performs itself.
    Inspection {
        text: String,
        source: Option<SourceRef>,
    },
}
impl Step {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Instruction { .. } => "instruction",
            Self::Program { .. } => "program",
            Self::Inspection { .. } => "inspection",
        }
    }
    pub fn program(&self) -> Option<&str> {
        match self {
            Self::Program { program } => Some(program),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillCapsule {
    pub form: Form,
    pub kind: CapsuleKind,
    pub mu: Mu,
    pub skills: Vec<SourceSkill>,
    pub source_edition: Option<String>,
    pub tool_chain_sha256: Option<String>,
    pub capabilities: Capabilities,
    pub bindings: Vec<Binding>,
    pub programs: Vec<Program>,
    pub steps: Vec<Step>,
    pub verification: Vec<Step>,
}

/// Observed values `E`. They answer for the agent while a capsule qualifies
/// and are never mounted for a warm run.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Examples {
    /// Values of the bindings and of what instruction steps produce.
    pub bindings: Map<String, Value>,
    /// The observed result of a program step, by plan index.
    pub steps: BTreeMap<usize, Map<String, Value>>,
}

/// Whether `value` is a scalar or a flat list of scalars (strings, numbers,
/// booleans). Nulls, objects and nested lists are refused.
pub fn is_admitted_value(value: &Value) -> bool {
    match value {
        Value::Null | Value::Object(_) => false,
        Value::Array(items) => items
            .iter()
            .all(|item| item.is_string() || item.is_number() || item.is_boolean()),
        _ => true,
    }
}

/// `[a-z][a-z0-9_]{0,63}`: bindings, outputs and program identifiers.
pub fn is_name(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 64
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
/// `[a-z0-9][a-z0-9-]{0,63}`: the name a capsule is listed under.
pub fn is_title(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 64
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
/// A bare interpreter name resolved on `PATH`, never a path.
pub fn is_runtime(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= 64
        && bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}
/// A `/`-separated path of plain components below some root: no `..`, no
/// drive, no leading separator, no backslash.
pub fn is_relative_path(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= ARGUMENT_BYTES
        && !text.contains(['\\', ':', '\0'])
        && text
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.ends_with(' '))
}
fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

struct Reader<'a> {
    violations: &'a mut Vec<Violation>,
}
impl Reader<'_> {
    fn refuse(&mut self, code: &'static str, at: &str, message: impl Into<String>) {
        self.violations.push(Violation::new(code, at, message));
    }
    /// The members of an object that has only `known` fields.
    fn object<'v>(
        &mut self,
        value: &'v Value,
        at: &str,
        known: &[&str],
    ) -> Option<&'v Map<String, Value>> {
        let Some(map) = value.as_object() else {
            self.refuse("invalid_type", at, "Expected a JSON object.");
            return None;
        };
        for key in map.keys() {
            if !known.contains(&key.as_str()) {
                let place = join(at, key);
                if key == "examples" {
                    self.refuse(
                        "examples_in_capsule",
                        &place,
                        "Observed values belong in the examples document, never in the capsule.",
                    );
                } else {
                    self.refuse("unknown_field", &place, format!("Unknown field `{key}`."));
                }
            }
        }
        Some(map)
    }
    fn required<'v>(
        &mut self,
        map: &'v Map<String, Value>,
        key: &str,
        at: &str,
    ) -> Option<&'v Value> {
        let value = map.get(key).filter(|value| !value.is_null());
        if value.is_none() {
            self.refuse(
                "missing_field",
                &join(at, key),
                format!("Field `{key}` is required."),
            );
        }
        value
    }
    fn text(&mut self, value: &Value, at: &str, limit: usize) -> Option<String> {
        let Some(text) = value.as_str() else {
            self.refuse("invalid_type", at, "Expected a string.");
            return None;
        };
        if text.trim().is_empty() {
            self.refuse("invalid_value", at, "The text is empty.");
            return None;
        }
        if text.chars().count() > limit || text.contains('\0') {
            self.refuse(
                "size_limit",
                at,
                format!("The text exceeds {limit} characters."),
            );
            return None;
        }
        Some(text.to_string())
    }
    fn required_text(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        at: &str,
        limit: usize,
    ) -> Option<String> {
        let value = self.required(map, key, at)?;
        self.text(value, &join(at, key), limit)
    }
    fn optional_text(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        at: &str,
        limit: usize,
    ) -> Option<String> {
        let value = map.get(key).filter(|value| !value.is_null())?;
        self.text(value, &join(at, key), limit)
    }
    fn array<'v>(&mut self, value: &'v Value, at: &str, limit: usize) -> Option<&'v Vec<Value>> {
        let Some(items) = value.as_array() else {
            self.refuse("invalid_type", at, "Expected a JSON array.");
            return None;
        };
        if items.len() > limit {
            self.refuse(
                "size_limit",
                at,
                format!("The list has more than {limit} items."),
            );
            return None;
        }
        Some(items)
    }
    /// A list of distinct names; an absent field is an empty list.
    fn names(&mut self, map: &Map<String, Value>, key: &str, at: &str) -> Vec<String> {
        let place = join(at, key);
        let Some(value) = map.get(key).filter(|value| !value.is_null()) else {
            return Vec::new();
        };
        let Some(items) = self.array(value, &place, NAMES) else {
            return Vec::new();
        };
        let mut names = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let here = format!("{place}[{index}]");
            match item.as_str() {
                Some(name) if is_name(name) => {
                    if names.iter().any(|known| known == name) {
                        self.refuse(
                            "duplicate_name",
                            &here,
                            format!("`{name}` is listed twice."),
                        );
                    } else {
                        names.push(name.to_string());
                    }
                }
                _ => self.refuse(
                    "invalid_value",
                    &here,
                    "A name is lower-case letters, digits and `_`, starting with a letter.",
                ),
            }
        }
        names
    }
    fn arguments(&mut self, map: &Map<String, Value>, key: &str, at: &str) -> Vec<String> {
        let place = join(at, key);
        let Some(value) = self.required(map, key, at) else {
            return Vec::new();
        };
        let Some(items) = self.array(value, &place, ARGUMENTS) else {
            return Vec::new();
        };
        let mut arguments = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let here = format!("{place}[{index}]");
            match item.as_str() {
                Some(text)
                    if !text.is_empty()
                        && text.len() <= ARGUMENT_BYTES
                        && !text.contains(['\0', '\n', '\r']) =>
                {
                    arguments.push(text.to_string());
                }
                Some(_) => self.refuse(
                    "size_limit",
                    &here,
                    format!("An argument is one line of 1 to {ARGUMENT_BYTES} bytes."),
                ),
                None => self.refuse("invalid_type", &here, "Expected a string."),
            }
        }
        if arguments.len() != items.len() {
            return Vec::new();
        }
        arguments
    }
    fn source(&mut self, map: &Map<String, Value>, at: &str) -> Option<SourceRef> {
        let value = map.get("source").filter(|value| !value.is_null())?;
        let place = join(at, "source");
        let source = self.object(value, &place, &["path", "section"])?;
        let path = self.required_text(source, "path", &place, ARGUMENT_BYTES)?;
        if !is_relative_path(&path) {
            self.refuse(
                "invalid_value",
                &join(&place, "path"),
                "A source path is relative to the project and has no `..`.",
            );
            return None;
        }
        let section = self.optional_text(source, "section", &place, 200);
        Some(SourceRef { path, section })
    }
    fn step(&mut self, value: &Value, at: &str, verification: bool) -> Option<Step> {
        let kind = value
            .as_object()
            .and_then(|map| map.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "program" => {
                let map = self.object(value, at, &["kind", "program"])?;
                let program = self.required_text(map, "program", at, 64)?;
                if !is_name(&program) {
                    self.refuse(
                        "invalid_value",
                        &join(at, "program"),
                        "A step names a program by its identifier.",
                    );
                    return None;
                }
                Some(Step::Program { program })
            }
            "instruction" if !verification => {
                let map = self.object(value, at, &["kind", "text", "source", "produces"])?;
                let text = self.required_text(map, "text", at, TEXT_CHARS);
                let source = self.source(map, at);
                let produces = self.names(map, "produces", at);
                Some(Step::Instruction {
                    text: text?,
                    source,
                    produces,
                })
            }
            "inspection" if verification => {
                let map = self.object(value, at, &["kind", "text", "source"])?;
                let text = self.required_text(map, "text", at, TEXT_CHARS);
                let source = self.source(map, at);
                Some(Step::Inspection {
                    text: text?,
                    source,
                })
            }
            _ => {
                self.refuse(
                    "invalid_step_kind",
                    &join(at, "kind"),
                    if verification {
                        "A verification step is a `program` or an `inspection`."
                    } else {
                        "A plan step is a `program` or an `instruction`."
                    },
                );
                None
            }
        }
    }
    fn plan(
        &mut self,
        map: &Map<String, Value>,
        key: &str,
        verification: bool,
        limit: usize,
    ) -> Vec<Step> {
        let Some(value) = map.get(key).filter(|value| !value.is_null()) else {
            return Vec::new();
        };
        let Some(items) = self.array(value, key, limit) else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| self.step(item, &format!("{key}[{index}]"), verification))
            .collect()
    }
}
fn join(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_string()
    } else {
        format!("{at}.{key}")
    }
}

impl SkillCapsule {
    /// Reads `capsule.json`. Every structural refusal is reported, not only
    /// the first; a capsule is returned only when there was none.
    pub fn parse(bytes: &[u8]) -> Result<Self, Vec<Violation>> {
        let value =
            infra_core::strict_json::parse(bytes, CAPSULE_BYTES, 16, 20_000).map_err(|error| {
                vec![Violation::new(
                    if bytes.len() > CAPSULE_BYTES {
                        "size_limit"
                    } else {
                        "invalid_json"
                    },
                    "",
                    error.to_string(),
                )]
            })?;
        Self::from_value(&value)
    }
    pub fn from_value(value: &Value) -> Result<Self, Vec<Violation>> {
        let mut violations = Vec::new();
        let capsule = read_capsule(value, &mut violations);
        match capsule {
            Some(capsule) if violations.is_empty() => Ok(capsule),
            _ => {
                if violations.is_empty() {
                    violations.push(Violation::new(
                        "invalid_type",
                        "",
                        "The capsule could not be read.",
                    ));
                }
                Err(violations)
            }
        }
    }
    /// The program a step names.
    pub fn program(&self, id: &str) -> Option<&Program> {
        self.programs.iter().find(|program| program.id == id)
    }
    /// `steps` followed by `verification`: the order an executor follows.
    pub fn plan(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().chain(self.verification.iter())
    }
    pub fn plan_len(&self) -> usize {
        self.steps.len() + self.verification.len()
    }
    pub fn plan_step(&self, index: usize) -> Option<&Step> {
        self.plan().nth(index)
    }
    /// Whether plan index `index` belongs to `verification`.
    pub fn is_verification(&self, index: usize) -> bool {
        index >= self.steps.len()
    }
    pub fn binding(&self, name: &str) -> Option<&Binding> {
        self.bindings.iter().find(|binding| binding.name == name)
    }
    /// The declared outputs of plan step `index` that a later program reads.
    pub fn outputs_read_later(&self, index: usize) -> Vec<String> {
        let Some(program) = self
            .plan_step(index)
            .and_then(Step::program)
            .and_then(|id| self.program(id))
        else {
            return Vec::new();
        };
        program
            .outputs
            .iter()
            .filter(|output| {
                self.plan()
                    .skip(index + 1)
                    .filter_map(Step::program)
                    .filter_map(|id| self.program(id))
                    .any(|later| later.inputs.contains(output))
            })
            .cloned()
            .collect()
    }
    /// The one spelling written to disk and hashed.
    pub fn to_value(&self) -> Value {
        let source = |source: &Option<SourceRef>, step: &mut Map<String, Value>| {
            if let Some(source) = source {
                let mut value = Map::new();
                value.insert("path".into(), source.path.clone().into());
                if let Some(section) = &source.section {
                    value.insert("section".into(), section.clone().into());
                }
                step.insert("source".into(), value.into());
            }
        };
        let step = |step: &Step| -> Value {
            let mut value = Map::new();
            value.insert("kind".into(), step.kind().into());
            match step {
                Step::Program { program } => {
                    value.insert("program".into(), program.clone().into());
                }
                Step::Instruction {
                    text,
                    source: from,
                    produces,
                } => {
                    value.insert("text".into(), text.clone().into());
                    source(from, &mut value);
                    if !produces.is_empty() {
                        value.insert("produces".into(), json!(produces));
                    }
                }
                Step::Inspection { text, source: from } => {
                    value.insert("text".into(), text.clone().into());
                    source(from, &mut value);
                }
            }
            value.into()
        };
        let mut capsule = Map::new();
        capsule.insert("schema_version".into(), SCHEMA_VERSION.into());
        capsule.insert("form".into(), self.form.as_str().into());
        if self.kind != CapsuleKind::Skill {
            capsule.insert("kind".into(), self.kind.as_str().into());
        }
        capsule.insert(
            "mu".into(),
            json!({"name": self.mu.name, "description": self.mu.description}),
        );
        let mut source = Map::new();
        source.insert(
            "skills".into(),
            self.skills
                .iter()
                .map(|skill| {
                    json!({
                        "name": skill.name,
                        "path": skill.path,
                        "tree_sha256": skill.tree_sha256,
                    })
                })
                .collect::<Vec<_>>()
                .into(),
        );
        if let Some(edition) = &self.source_edition {
            source.insert("edition".into(), edition.clone().into());
        }
        if let Some(tree_sha256) = &self.tool_chain_sha256 {
            source.insert("tool_chain".into(), json!({"tree_sha256": tree_sha256}));
        }
        capsule.insert("source".into(), Value::Object(source));
        if self.form == Form::Textual {
            return capsule.into();
        }
        let mut capabilities = Map::new();
        capabilities.insert("network".into(), self.capabilities.network.as_str().into());
        capabilities.insert(
            "runtimes".into(),
            Value::Array(
                self.capabilities
                    .runtimes
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
        if !self.capabilities.allow_ignored.is_empty() {
            capabilities.insert(
                "allow_ignored".into(),
                Value::Array(
                    self.capabilities
                        .allow_ignored
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        capsule.insert("capabilities".into(), Value::Object(capabilities));
        capsule.insert(
            "bindings".into(),
            self.bindings
                .iter()
                .map(|binding| {
                    let mut value = Map::new();
                    value.insert("name".into(), binding.name.clone().into());
                    value.insert("type".into(), binding.kind.as_str().into());
                    value.insert("origin".into(), binding.origin.clone().into());
                    if let Some(description) = &binding.description {
                        value.insert("description".into(), description.clone().into());
                    }
                    Value::Object(value)
                })
                .collect::<Vec<_>>()
                .into(),
        );
        capsule.insert(
            "programs".into(),
            self.programs
                .iter()
                .map(|program| {
                    let mut value = Map::new();
                    value.insert("id".into(), program.id.clone().into());
                    if let Some(description) = &program.description {
                        value.insert("description".into(), description.clone().into());
                    }
                    value.insert("entry".into(), json!(program.entry));
                    value.insert("inputs".into(), json!(program.inputs));
                    value.insert("outputs".into(), json!(program.outputs));
                    value.insert("check".into(), json!(program.check));
                    value.insert("timeout_seconds".into(), program.timeout_seconds.into());
                    value.insert("read_only".into(), program.read_only.into());
                    Value::Object(value)
                })
                .collect::<Vec<_>>()
                .into(),
        );
        capsule.insert(
            "steps".into(),
            self.steps.iter().map(step).collect::<Vec<_>>().into(),
        );
        capsule.insert(
            "verification".into(),
            self.verification
                .iter()
                .map(step)
                .collect::<Vec<_>>()
                .into(),
        );
        capsule.into()
    }
    /// `capsule.json` as stored: indented, one trailing newline.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(&self.to_value())
            .expect("a capsule value has string keys and finite numbers");
        bytes.push(b'\n');
        bytes
    }
}

fn read_capsule(value: &Value, violations: &mut Vec<Violation>) -> Option<SkillCapsule> {
    let mut reader = Reader { violations };
    let form = match value.get("form").and_then(Value::as_str) {
        Some("hybrid") => Form::Hybrid,
        Some("textual") => Form::Textual,
        _ => {
            reader.object(value, "", &[])?;
            reader.refuse("invalid_value", "form", "`form` is `hybrid` or `textual`.");
            return None;
        }
    };
    let known: &[&str] = match form {
        Form::Hybrid => &[
            "schema_version",
            "form",
            "kind",
            "mu",
            "source",
            "capabilities",
            "bindings",
            "programs",
            "steps",
            "verification",
        ],
        Form::Textual => &["schema_version", "form", "kind", "mu", "source"],
    };
    let map = reader.object(value, "", known)?;
    let kind = match map.get("kind").filter(|value| !value.is_null()) {
        None => Some(CapsuleKind::Skill),
        Some(value) => match value.as_str().and_then(CapsuleKind::parse) {
            Some(kind) => Some(kind),
            None => {
                reader.refuse(
                    "invalid_value",
                    "kind",
                    "`kind` is `skill`, `tool_chain` or `template`.",
                );
                None
            }
        },
    };
    match map.get("schema_version").and_then(Value::as_u64) {
        Some(SCHEMA_VERSION) => {}
        Some(other) => reader.refuse(
            "schema_version_unsupported",
            "schema_version",
            format!("This engine reads schema version {SCHEMA_VERSION}, not {other}."),
        ),
        None => reader.refuse(
            "missing_field",
            "schema_version",
            format!("`schema_version` is {SCHEMA_VERSION}."),
        ),
    }
    let mu = reader.required(map, "mu", "").and_then(|value| {
        let mu = reader.object(value, "mu", &["name", "description"])?;
        let name = reader.required_text(mu, "name", "mu", 64);
        let description = reader.required_text(mu, "description", "mu", DESCRIPTION_CHARS);
        let name = name?;
        if !is_title(&name) {
            reader.refuse(
                "invalid_value",
                "mu.name",
                "A capsule name is lower-case letters, digits and `-`.",
            );
            return None;
        }
        Some(Mu {
            name,
            description: description?,
        })
    });
    let mut source_edition = None;
    let mut tool_chain_sha256 = None;
    let skills = reader.required(map, "source", "").and_then(|value| {
        let source = reader.object(value, "source", &["skills", "edition", "tool_chain"])?;
        source_edition = reader.optional_text(source, "edition", "source", 128);
        if let Some(chain) = source.get("tool_chain").filter(|v| !v.is_null())
            && let Some(chain_obj) = reader.object(chain, "source.tool_chain", &["tree_sha256"])
            && let Some(sha) =
                reader.required_text(chain_obj, "tree_sha256", "source.tool_chain", 64)
        {
            if is_sha256(&sha) {
                tool_chain_sha256 = Some(sha);
            } else {
                reader.refuse(
                    "invalid_value",
                    "source.tool_chain.tree_sha256",
                    "`tree_sha256` is 64 lower-case hexadecimal digits.",
                );
            }
        }
        let items = reader.required(source, "skills", "source")?;
        let items = reader.array(items, "source.skills", SOURCE_SKILLS)?;
        if items.is_empty() && kind != Some(CapsuleKind::ToolChain) {
            reader.refuse(
                "missing_field",
                "source.skills",
                "A capsule names the skill it was compiled from.",
            );
            return None;
        }
        let mut skills: Vec<SourceSkill> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let at = format!("source.skills[{index}]");
            let Some(skill) = reader.object(item, &at, &["name", "path", "tree_sha256"]) else {
                continue;
            };
            let name = reader.required_text(skill, "name", &at, 128);
            let path = reader.required_text(skill, "path", &at, ARGUMENT_BYTES);
            let digest = reader.required_text(skill, "tree_sha256", &at, 64);
            let (Some(name), Some(path), Some(digest)) = (name, path, digest) else {
                continue;
            };
            if !is_relative_path(&path) {
                reader.refuse(
                    "invalid_value",
                    &join(&at, "path"),
                    "A skill path is relative to the project and has no `..`.",
                );
                continue;
            }
            if !is_sha256(&digest) {
                reader.refuse(
                    "invalid_value",
                    &join(&at, "tree_sha256"),
                    "`tree_sha256` is 64 lower-case hexadecimal digits.",
                );
                continue;
            }
            if skills.iter().any(|known| known.path == path) {
                reader.refuse(
                    "duplicate_name",
                    &join(&at, "path"),
                    "A source skill is listed twice.",
                );
                continue;
            }
            skills.push(SourceSkill {
                name,
                path,
                tree_sha256: digest,
            });
        }
        (skills.len() == items.len()).then_some(skills)
    });
    if form == Form::Textual {
        return Some(SkillCapsule {
            form,
            kind: kind?,
            mu: mu?,
            skills: skills?,
            source_edition,
            tool_chain_sha256,
            capabilities: Capabilities {
                network: Network::None,
                runtimes: Vec::new(),
                allow_ignored: Vec::new(),
            },
            bindings: Vec::new(),
            programs: Vec::new(),
            steps: Vec::new(),
            verification: Vec::new(),
        });
    }
    let capabilities = reader.required(map, "capabilities", "").and_then(|value| {
        let capabilities = reader.object(
            value,
            "capabilities",
            &["network", "runtimes", "allow_ignored"],
        )?;
        let network = match capabilities.get("network").and_then(Value::as_str) {
            Some("none") => Some(Network::None),
            Some("required") => Some(Network::Required),
            _ => {
                reader.refuse(
                    "invalid_value",
                    "capabilities.network",
                    "`network` is `none` or `required`.",
                );
                None
            }
        };
        let mut runtimes = Vec::new();
        if let Some(items) = reader
            .required(capabilities, "runtimes", "capabilities")
            .and_then(|items| reader.array(items, "capabilities.runtimes", RUNTIMES))
        {
            for (index, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(name)
                        if is_runtime(name) && !runtimes.iter().any(|known| known == name) =>
                    {
                        runtimes.push(name.to_string());
                    }
                    _ => reader.refuse(
                        "invalid_value",
                        &format!("capabilities.runtimes[{index}]"),
                        "A runtime is a distinct bare interpreter name such as `python3`.",
                    ),
                }
            }
        }
        let mut allow_ignored = Vec::new();
        if let Some(items) = capabilities
            .get("allow_ignored")
            .filter(|v| !v.is_null())
            .and_then(|items| reader.array(items, "capabilities.allow_ignored", RUNTIMES))
        {
            for (index, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(name)
                        if crate::executor::is_safe_allowlist_name(name)
                            && !allow_ignored.iter().any(|known| known == name) =>
                    {
                        allow_ignored.push(name.to_string());
                    }
                    _ => reader.refuse(
                        "invalid_value",
                        &format!("capabilities.allow_ignored[{index}]"),
                        "An allowed entry must be a distinct non-sensitive folder or file name.",
                    ),
                }
            }
        }
        Some(Capabilities {
            network: network?,
            runtimes,
            allow_ignored,
        })
    });
    let mut bindings = Vec::new();
    if let Some(items) = map
        .get("bindings")
        .filter(|value| !value.is_null())
        .and_then(|value| reader.array(value, "bindings", BINDINGS))
    {
        for (index, item) in items.iter().enumerate() {
            let at = format!("bindings[{index}]");
            let Some(binding) =
                reader.object(item, &at, &["name", "type", "origin", "description"])
            else {
                continue;
            };
            let name = reader.required_text(binding, "name", &at, 64);
            let description = reader.optional_text(binding, "description", &at, DESCRIPTION_CHARS);
            let kind = match binding.get("type").and_then(Value::as_str) {
                Some("string") => Some(BindingType::String),
                Some("path") => Some(BindingType::Path),
                Some("number") => Some(BindingType::Number),
                Some("boolean") => Some(BindingType::Boolean),
                Some("list") => Some(BindingType::List),
                _ => {
                    reader.refuse(
                        "invalid_value",
                        &join(&at, "type"),
                        "`type` is `string`, `path`, `number`, `boolean` or `list`.",
                    );
                    None
                }
            };
            let origin = match binding.get("origin").and_then(Value::as_str) {
                Some("request") => Some("request".to_string()),
                _ => {
                    reader.refuse(
                        "invalid_value",
                        &join(&at, "origin"),
                        "`origin` is `request`: a binding is a value the request states.",
                    );
                    None
                }
            };
            let Some(name) = name else { continue };
            if !is_name(&name) {
                reader.refuse(
                    "invalid_value",
                    &join(&at, "name"),
                    "A name is lower-case letters, digits and `_`, starting with a letter.",
                );
                continue;
            }
            if let (Some(kind), Some(origin)) = (kind, origin) {
                bindings.push(Binding {
                    name,
                    kind,
                    origin,
                    description,
                });
            }
        }
    }
    let mut programs = Vec::new();
    if let Some(items) = map
        .get("programs")
        .filter(|value| !value.is_null())
        .and_then(|value| reader.array(value, "programs", PROGRAMS))
    {
        for (index, item) in items.iter().enumerate() {
            let at = format!("programs[{index}]");
            let Some(program) = reader.object(
                item,
                &at,
                &[
                    "id",
                    "description",
                    "entry",
                    "inputs",
                    "outputs",
                    "check",
                    "timeout_seconds",
                    "read_only",
                ],
            ) else {
                continue;
            };
            let id = reader.required_text(program, "id", &at, 64);
            let description = reader.optional_text(program, "description", &at, DESCRIPTION_CHARS);
            let entry = reader.arguments(program, "entry", &at);
            let check = reader.arguments(program, "check", &at);
            let inputs = reader.names(program, "inputs", &at);
            let outputs = reader.names(program, "outputs", &at);
            let timeout = match program.get("timeout_seconds").and_then(Value::as_u64) {
                Some(seconds) if TIMEOUT_SECONDS.contains(&seconds) => Some(seconds),
                _ => {
                    reader.refuse(
                        "invalid_value",
                        &join(&at, "timeout_seconds"),
                        format!(
                            "`timeout_seconds` is a whole number from {} to {}.",
                            TIMEOUT_SECONDS.start(),
                            TIMEOUT_SECONDS.end()
                        ),
                    );
                    None
                }
            };
            let read_only = match program.get("read_only").and_then(Value::as_bool) {
                Some(read_only) => Some(read_only),
                None => {
                    reader.refuse(
                        "missing_field",
                        &join(&at, "read_only"),
                        "`read_only` states whether the program changes the workspace.",
                    );
                    None
                }
            };
            let Some(id) = id else { continue };
            if !is_name(&id) {
                reader.refuse(
                    "invalid_value",
                    &join(&at, "id"),
                    "A program identifier is lower-case letters, digits and `_`.",
                );
                continue;
            }
            if let (Some(timeout_seconds), Some(read_only)) = (timeout, read_only) {
                programs.push(Program {
                    id,
                    entry,
                    inputs,
                    outputs,
                    check,
                    timeout_seconds,
                    read_only,
                    description,
                });
            }
        }
    }
    let steps = reader.plan(map, "steps", false, PLAN_STEPS);
    let verification = reader.plan(map, "verification", true, PLAN_STEPS);
    if steps.len() + verification.len() > PLAN_STEPS {
        reader.refuse(
            "size_limit",
            "steps",
            format!("A plan has at most {PLAN_STEPS} steps, verification included."),
        );
    }
    Some(SkillCapsule {
        form,
        kind: kind?,
        mu: mu?,
        skills: skills?,
        source_edition,
        tool_chain_sha256,
        capabilities: capabilities?,
        bindings,
        programs,
        steps,
        verification,
    })
}

impl Examples {
    /// Reads `examples.json`: `{"bindings": {...}, "steps": {"<index>": {"result": {...}}}}`.
    pub fn parse(bytes: &[u8]) -> Result<Self, Vec<Violation>> {
        let value =
            infra_core::strict_json::parse(bytes, EXAMPLES_BYTES, 16, 50_000).map_err(|error| {
                vec![Violation::new(
                    if bytes.len() > EXAMPLES_BYTES {
                        "size_limit"
                    } else {
                        "invalid_json"
                    },
                    "examples",
                    error.to_string(),
                )]
            })?;
        Self::from_value(&value)
    }
    pub fn from_value(value: &Value) -> Result<Self, Vec<Violation>> {
        let mut violations = Vec::new();
        let mut reader = Reader {
            violations: &mut violations,
        };
        let mut examples = Examples::default();
        if let Some(map) = reader.object(value, "examples", &["bindings", "steps"]) {
            if let Some(bindings) = map.get("bindings").filter(|value| !value.is_null()) {
                match bindings.as_object() {
                    Some(bindings) => {
                        for (name, value) in bindings {
                            let at = format!("examples.bindings.{name}");
                            if !is_name(name) {
                                reader.refuse("invalid_value", &at, "An example names a binding.");
                            } else if !is_admitted_value(value) {
                                reader.refuse(
                                    "invalid_type",
                                    &at,
                                    "An observed value is a scalar or a list of scalars.",
                                );
                            } else {
                                examples.bindings.insert(name.clone(), value.clone());
                            }
                        }
                    }
                    None => reader.refuse(
                        "invalid_type",
                        "examples.bindings",
                        "Expected a JSON object.",
                    ),
                }
            }
            if let Some(steps) = map.get("steps").filter(|value| !value.is_null()) {
                match steps.as_object() {
                    Some(steps) => {
                        for (index, value) in steps {
                            let at = format!("examples.steps.{index}");
                            let position = index
                                .parse::<usize>()
                                .ok()
                                .filter(|position| position.to_string() == *index);
                            let result = reader
                                .object(value, &at, &["result"])
                                .and_then(|step| step.get("result"))
                                .and_then(Value::as_object);
                            match (position, result) {
                                (Some(position), Some(result)) => {
                                    examples.steps.insert(position, result.clone());
                                }
                                (None, _) => reader.refuse(
                                    "invalid_value",
                                    &at,
                                    "An observed result is keyed by its plan index.",
                                ),
                                (_, None) => reader.refuse(
                                    "missing_field",
                                    &join(&at, "result"),
                                    "An observed step carries the program's `result` object.",
                                ),
                            }
                        }
                    }
                    None => {
                        reader.refuse("invalid_type", "examples.steps", "Expected a JSON object.")
                    }
                }
            }
        }
        if violations.is_empty() {
            Ok(examples)
        } else {
            Err(violations)
        }
    }
    pub fn to_value(&self) -> Value {
        let steps = self
            .steps
            .iter()
            .map(|(index, result)| (index.to_string(), json!({"result": result})))
            .collect::<Map<_, _>>();
        json!({"bindings": self.bindings, "steps": steps})
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes =
            serde_json::to_vec_pretty(&self.to_value()).expect("observed values are plain JSON");
        bytes.push(b'\n');
        bytes
    }
}
