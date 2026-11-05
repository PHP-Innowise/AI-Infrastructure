//! Inventory and indexing share one admitted Brain source image.
use infra_core::{
    BrainRepository, CreateRecord, ErrorKind, RecordImage, Roots, RuntimeContext, RuntimeError,
    domain, index, query,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    root: PathBuf,
    context: RuntimeContext,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("infra-brain-image-{}", domain::uuid4()));
        fs::create_dir_all(root.join("project-brain/schemas")).unwrap();
        // The runtime reads the canonical root: on Windows TEMP may name it by
        // its short 8.3 form.
        let root = infra_core::fs::canonicalize(root).unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../PHP Core/project-brain/schemas/dynamic-record.schema.json"),
            root.join("project-brain/schemas/dynamic-record.schema.json"),
        )
        .unwrap();
        let context =
            RuntimeContext::new(Roots::installed(&root).unwrap(), "local", "governed").unwrap();
        Self { root, context }
    }

    fn brain(&self) -> BrainRepository {
        BrainRepository::new(self.context.clone()).unwrap()
    }

    fn task(&self) -> Value {
        self.brain()
            .create(CreateRecord {
                external_id: "TASK-HELD-IMAGE".into(),
                title: "Keep one admitted source image".into(),
                ..CreateRecord::default()
            })
            .unwrap()
    }

    fn write(&self, relative: &str, metadata: &Value, body: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            domain::render_markdown_record(metadata, body).unwrap(),
        )
        .unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// Preserve the materialized lookup as a reference for the streaming projection.
fn inventory_find(
    brain: &BrainRepository,
    identifier: &str,
    record_type: Option<&str>,
    include_archive: bool,
) -> infra_core::Result<RecordImage> {
    let inventory = brain.inventory(include_archive)?;
    let matches: Vec<_> = inventory
        .records
        .into_iter()
        .filter(|image| {
            record_type.is_none_or(|kind| image.record["type"].as_str() == Some(kind))
                && (image.record["id"].as_str() == Some(identifier)
                    || image.record["external_id"].as_str() == Some(identifier))
        })
        .collect();
    let claimants: Vec<_> = inventory
        .problems
        .iter()
        .filter(|problem| {
            (problem.id.as_deref() == Some(identifier)
                || problem.external_id.as_deref() == Some(identifier))
                && record_type.is_none_or(|kind| {
                    problem
                        .record_type
                        .as_deref()
                        .is_none_or(|actual| actual == kind)
                })
        })
        .collect();
    if matches.len() + claimants.len() > 1 {
        return Err(RuntimeError::conflict(format!(
            "Project Brain record identifier is ambiguous: {identifier}"
        )));
    }
    if let Some(problem) = claimants.first() {
        let shown = problem
            .path
            .strip_prefix(&brain.roots().state)
            .ok()
            .and_then(Path::to_str)
            .map(|relative| relative.replace('\\', "/"))
            .unwrap_or_else(|| {
                problem
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        return Err(RuntimeError::invalid(format!(
            "Project Brain record invalid: {shown}: {}",
            problem.error
        )));
    }
    matches.into_iter().next().ok_or_else(|| {
        RuntimeError::not_found(format!(
            "Project Brain {} not found: {identifier}",
            record_type.unwrap_or("record")
        ))
    })
}

fn find_outcome(result: infra_core::Result<RecordImage>) -> Value {
    match result {
        Ok(image) => json!({
            "path": image.path,
            "record": image.record,
            "body": image.body,
            "archived": image.archived,
        }),
        Err(error) => json!({
            "kind": format!("{:?}", error.kind),
            "message": error.message,
            "code": error.code,
            "os_code": error.os_code,
        }),
    }
}

fn assert_inventory_find_parity(brain: &BrainRepository, identifiers: &[&str]) {
    for identifier in identifiers {
        for record_type in [None, Some("task"), Some("finding")] {
            for include_archive in [false, true] {
                assert_eq!(
                    find_outcome(brain.find(identifier, record_type, include_archive)),
                    find_outcome(inventory_find(
                        brain,
                        identifier,
                        record_type,
                        include_archive
                    )),
                    "identifier={identifier:?}, type={record_type:?}, archive={include_archive}",
                );
            }
        }
    }
}

fn derived_record(task: &Value, kind: &str, external_id: &str) -> Value {
    let mut record = task.clone();
    let state = domain::initial_state(kind).unwrap();
    record["id"] = json!(domain::uuid4());
    record["type"] = json!(kind);
    record["external_id"] = json!(external_id);
    record["status"] = json!(state);
    record["transitions"] = json!([{
        "from": null, "to": state, "at": task["created_at"],
        "actor": "local", "reason": "Fixture created",
    }]);
    record.as_object_mut().unwrap().remove("phase");
    domain::validate_record(&record, false).unwrap();
    record
}

#[test]
fn inventory_preserves_archived_and_misplaced_handoff_classification() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = fixture.task();
    let task_id = task["id"].as_str().unwrap();
    let active = brain.handoff(task_id).unwrap();
    let mut closed = active.clone();
    closed["status"] = json!("closed");
    let archived = fixture.write(
        &format!("project-brain/archive/handoffs/{task_id}.md"),
        &closed,
        "Closed handoff",
    );
    let misplaced = fixture.write(
        "project-brain/dynamic/handoff-copy.md",
        &active,
        "Misplaced handoff",
    );
    let wrong_name = fixture.write(
        "project-brain/archive/handoffs/wrong-name.md",
        &closed,
        "Wrong archive name",
    );

    let dynamic = brain.inventory(false).unwrap();
    assert_eq!(dynamic.records.len(), 1);
    assert_eq!(dynamic.records[0].record, task);
    assert_eq!(dynamic.problems.len(), 1);
    assert_eq!(dynamic.problems[0].path, misplaced);

    let all = brain.inventory(true).unwrap();
    assert_eq!(all.records.len(), 1);
    assert_eq!(all.problems.len(), 2);
    for path in [&misplaced, &wrong_name] {
        let problem = all
            .problems
            .iter()
            .find(|problem| &problem.path == path)
            .unwrap();
        assert_eq!(problem.external_id.as_deref(), Some(task_id));
        assert_eq!(problem.record_type, None);
        assert_eq!(problem.error, "unexpected handoff in authority path");
    }
    assert!(all.problems.iter().all(|problem| problem.path != archived));

    closed["revision"] = json!(0);
    fixture.write(
        &format!("project-brain/archive/handoffs/{task_id}.md"),
        &closed,
        "Invalid archived revision",
    );
    assert!(
        brain
            .inventory(true)
            .unwrap()
            .problems
            .iter()
            .any(|problem| problem.path == archived)
    );
}

#[test]
fn invalid_metadata_and_malformed_filename_claimants_still_block_lookup() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = fixture.task();
    let task_id = task["id"].as_str().unwrap();
    let mut invalid = task.clone();
    invalid["revision"] = json!(0);
    let claimant = fixture.write(
        &format!("project-brain/dynamic/claimants/{task_id}.md"),
        &invalid,
        "Invalid claimant",
    );
    let inventory = brain.inventory(false).unwrap();
    assert_eq!(inventory.problems.len(), 1);
    assert_eq!(inventory.problems[0].id.as_deref(), Some(task_id));
    assert_eq!(
        inventory.problems[0].external_id.as_deref(),
        Some("TASK-HELD-IMAGE")
    );
    assert_eq!(
        brain.get("TASK-HELD-IMAGE").unwrap_err().kind,
        ErrorKind::Conflict
    );

    fs::write(&claimant, "---\nnot JSON\n---\n").unwrap();
    let inventory = brain.inventory(false).unwrap();
    assert_eq!(inventory.problems[0].id.as_deref(), Some(task_id));
    assert_eq!(inventory.problems[0].external_id, None);
    assert_eq!(brain.get(task_id).unwrap_err().kind, ErrorKind::Conflict);
    fs::remove_file(brain.dynamic_path(&task).unwrap()).unwrap();
    assert_eq!(
        brain.get(task_id).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn discovery_hashes_exact_raw_bytes_and_excludes_body_only_secrets() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = fixture.task();
    let path = brain.dynamic_path(&task).unwrap();
    let raw = format!(
        "---\n{}\n---\n\n\n# Held image\nKeep trailing spaces.  \n",
        serde_json::to_string(&task).unwrap()
    );
    fs::write(&path, &raw).unwrap();
    let key = path
        .strip_prefix(&fixture.root)
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let discovery = index::discover_sources(&fixture.context, &BTreeMap::new()).unwrap();
    let document = discovery
        .documents
        .iter()
        .find(|document| document.path == key)
        .unwrap();
    assert_eq!(document.source_hash, query::sha256(raw.as_bytes()));
    assert_eq!(document.content, "# Held image\nKeep trailing spaces.  \n");
    assert_eq!(brain.get(task["id"].as_str().unwrap()).unwrap(), task);

    let secret_raw = format!("{raw}\n-----BEGIN PRIVATE KEY-----\nfixture-only\n");
    fs::write(&path, secret_raw).unwrap();
    let discovery = index::discover_sources(&fixture.context, &BTreeMap::new()).unwrap();
    assert!(
        discovery
            .documents
            .iter()
            .all(|document| document.path != key)
    );
    assert!(
        discovery
            .excluded
            .iter()
            .any(|excluded| excluded.path == key && excluded.reason == "secret")
    );
    assert_eq!(
        brain.inventory(false).unwrap().records.len(),
        1,
        "the body-only secret is excluded by indexing, not mistaken for malformed metadata"
    );
}

#[test]
fn metadata_exclusion_precedes_body_only_secret_detection() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let mut task = fixture.task();
    let path = brain.dynamic_path(&task).unwrap();
    task["privacy"] = json!("private");
    fs::write(
        &path,
        domain::render_markdown_record(&task, "-----BEGIN PRIVATE KEY-----\nfixture-only").unwrap(),
    )
    .unwrap();
    let key = path
        .strip_prefix(&fixture.root)
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let discovery = index::discover_sources(&fixture.context, &BTreeMap::new()).unwrap();
    assert!(
        discovery
            .documents
            .iter()
            .all(|document| document.path != key)
    );
    let excluded = discovery
        .excluded
        .iter()
        .find(|excluded| excluded.path == key)
        .unwrap();
    assert_eq!(excluded.reason, "privacy");
}

#[test]
fn find_matches_materialized_images_and_rereads_external_changes() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let mut task = fixture.task();
    let path = brain.dynamic_path(&task).unwrap();
    task["phase"] = json!("execution");
    let body = "# Lookup café 🦀\nKeep trailing spaces.  \n";
    let raw = format!(
        "---\r\n{}\r\n---\r\n\r\n\r\n{}",
        serde_json::to_string(&task).unwrap(),
        body.replace('\n', "\r\n")
    );
    fs::write(&path, raw).unwrap();
    let unrelated = derived_record(&task, "task", "TASK-UNRELATED");
    fixture.write(
        "project-brain/dynamic/zz-unrelated.md",
        &unrelated,
        &"Unrelated body\n".repeat(4096),
    );
    fs::write(
        fixture.root.join("project-brain/dynamic/zz-malformed.md"),
        "---\nnot JSON\n---\nUnrelated invalid source\n",
    )
    .unwrap();

    let task_id = task["id"].as_str().unwrap().to_owned();
    let image = brain.find(&task_id, Some("task"), false).unwrap();
    task["phase"] = json!("implementation");
    assert_eq!(image.path, path);
    assert_eq!(image.record, task);
    assert_eq!(image.body, body);
    assert!(!image.archived);
    assert_inventory_find_parity(&brain, &[&task_id, "TASK-HELD-IMAGE", "missing"]);

    task["revision"] = json!(2);
    task["progress"] = json!("Changed outside the repository API");
    let changed_body = "# Fresh image\nKeep new trailing spaces.  \n";
    fs::write(
        &path,
        format!(
            "---\n{}\n---\n\n{changed_body}",
            serde_json::to_string(&task).unwrap()
        ),
    )
    .unwrap();
    let fresh = brain.find(&task_id, None, false).unwrap();
    assert_eq!(fresh.path, path);
    assert_eq!(fresh.record, task);
    assert_eq!(fresh.body, changed_body);
    assert_inventory_find_parity(&brain, &[&task_id, "TASK-HELD-IMAGE"]);
}

#[test]
fn find_preserves_typed_and_untyped_duplicate_resolution() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = fixture.task();
    let alias = task["external_id"].as_str().unwrap();
    let finding = derived_record(&task, "finding", alias);
    let finding_path = fixture.write(
        "project-brain/dynamic/findings/shared-alias.md",
        &finding,
        "Finding body",
    );

    assert_eq!(brain.get(alias).unwrap_err().kind, ErrorKind::Conflict);
    assert_eq!(brain.get_task(alias).unwrap(), task);
    let found = brain.find(alias, Some("finding"), false).unwrap();
    assert_eq!(found.path, finding_path);
    assert_eq!(found.record, finding);
    assert_eq!(found.body, "Finding body\n");
    assert_eq!(
        brain
            .find(finding["id"].as_str().unwrap(), Some("task"), false)
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_inventory_find_parity(
        &brain,
        &[
            alias,
            task["id"].as_str().unwrap(),
            finding["id"].as_str().unwrap(),
        ],
    );

    let duplicate = derived_record(&task, "task", alias);
    let duplicate_path = fixture.write(
        "project-brain/dynamic/zz-duplicate-task.md",
        &duplicate,
        "Later duplicate",
    );
    assert_eq!(brain.get_task(alias).unwrap_err().kind, ErrorKind::Conflict);
    assert_inventory_find_parity(&brain, &[alias, duplicate["id"].as_str().unwrap()]);

    fs::write(
        duplicate_path,
        domain::render_markdown_record(&task, "Duplicate UUID and alias").unwrap(),
    )
    .unwrap();
    assert_eq!(
        brain
            .get_task(task["id"].as_str().unwrap())
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert_inventory_find_parity(&brain, &[alias, task["id"].as_str().unwrap()]);
}

#[test]
fn find_keeps_wrong_kind_and_unknown_kind_invalid_claimants() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = fixture.task();
    let task_id = task["id"].as_str().unwrap();
    let alias = task["external_id"].as_str().unwrap();
    let mut invalid_finding = derived_record(&task, "finding", alias);
    invalid_finding["revision"] = json!(0);
    fixture.write(
        "project-brain/dynamic/zz-claimants/wrong-kind.md",
        &invalid_finding,
        "Invalid finding with the task alias",
    );
    assert_eq!(brain.get_task(alias).unwrap(), task);
    assert_eq!(brain.get(alias).unwrap_err().kind, ErrorKind::Conflict);
    let refused = brain
        .find(alias, Some("finding"), false)
        .unwrap_err()
        .message;
    assert!(
        refused.starts_with(
            &"Project Brain record invalid: project-brain/dynamic/zz-claimants/wrong-kind.md: "
                .replace('/', std::path::MAIN_SEPARATOR_STR)
        ),
        "{refused}"
    );
    assert_inventory_find_parity(&brain, &[alias, task_id]);

    let unknown = fixture
        .root
        .join(format!("project-brain/dynamic/zz-claimants/{task_id}.md"));
    fs::write(&unknown, "---\nmalformed JSON\n---\nUnknown record type\n").unwrap();
    assert_eq!(
        brain.get_task(task_id).unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_inventory_find_parity(&brain, &[alias, task_id]);

    fs::remove_file(brain.dynamic_path(&task).unwrap()).unwrap();
    let invalid = brain.get_task(task_id).unwrap_err();
    assert_eq!(invalid.kind, ErrorKind::InvalidInput);
    // Separators follow how each side spelled the path, not the record.
    assert_eq!(
        invalid.message.replace('\\', "/"),
        format!(
            "Project Brain record invalid: {}: invalid JSON frontmatter",
            unknown.strip_prefix(&fixture.root).unwrap().display()
        )
        .replace('\\', "/")
    );
    let second_unknown = fixture.root.join("project-brain/dynamic/zz-more");
    fs::create_dir_all(&second_unknown).unwrap();
    fs::write(
        second_unknown.join(format!("{task_id}.md")),
        "not frontmatter",
    )
    .unwrap();
    assert_eq!(
        brain.get_task(task_id).unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_inventory_find_parity(&brain, &[alias, task_id, "missing"]);
}

#[test]
fn find_preserves_archive_scope_and_handoff_claimants() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = fixture.task();
    let task_id = task["id"].as_str().unwrap();
    let mut archived = derived_record(&task, "task", "TASK-ARCHIVED");
    archived["status"] = json!("completed");
    archived["transitions"].as_array_mut().unwrap().push(json!({
        "from": "active", "to": "completed", "at": task["updated_at"],
        "actor": "local", "reason": "Fixture completed",
    }));
    domain::validate_record(&archived, true).unwrap();
    let archived_path = fixture.write(
        "project-brain/archive/task/completed.md",
        &archived,
        "Archived body",
    );
    let mut handoff = brain.handoff(task_id).unwrap();
    handoff["status"] = json!("closed");
    fixture.write(
        &format!("project-brain/archive/handoffs/{task_id}.md"),
        &handoff,
        "Valid closed archive handoff",
    );
    assert_eq!(
        brain.find("TASK-ARCHIVED", None, false).unwrap_err().kind,
        ErrorKind::NotFound
    );
    let found = brain.find("TASK-ARCHIVED", Some("task"), true).unwrap();
    assert_eq!(found.path, archived_path);
    assert_eq!(found.record, archived);
    assert_eq!(found.body, "Archived body\n");
    assert!(found.archived);
    assert_eq!(
        brain.find(task_id, Some("task"), true).unwrap().record,
        task
    );
    assert_inventory_find_parity(&brain, &[task_id, "TASK-ARCHIVED"]);

    archived["external_id"] = task["external_id"].clone();
    fixture.write(
        "project-brain/archive/task/completed.md",
        &archived,
        "Archived alias duplicate",
    );
    assert_eq!(brain.get_task("TASK-HELD-IMAGE").unwrap(), task);
    assert_eq!(
        brain
            .find("TASK-HELD-IMAGE", Some("task"), true)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    fixture.write(
        "project-brain/archive/handoffs/wrong-name.md",
        &handoff,
        "Misplaced archive handoff claimant",
    );
    assert_eq!(brain.get_task(task_id).unwrap(), task);
    assert_eq!(
        brain.find(task_id, Some("task"), true).unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_inventory_find_parity(&brain, &[task_id, "TASK-HELD-IMAGE"]);
}

#[cfg(unix)]
#[test]
fn find_does_not_return_before_later_inventory_admission_errors() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = fixture.task();
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(brain.get_task(task_id).unwrap(), task);
    let late_directory = fixture.root.join("project-brain/dynamic/zz-late");
    fs::create_dir_all(&late_directory).unwrap();
    std::os::unix::fs::symlink(
        brain.dynamic_path(&task).unwrap(),
        late_directory.join("linked.md"),
    )
    .unwrap();

    let failure = brain.get_task(task_id).unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Policy);
    assert_eq!(failure.message, "Linked Project Brain inventory entry");
    assert_inventory_find_parity(&brain, &[task_id, "TASK-HELD-IMAGE", "missing"]);
}
