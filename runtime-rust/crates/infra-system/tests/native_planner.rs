//! Native read-only system catalog/planner contracts; no provider is needed.
use infra_system::{catalog::System, common, execute};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-system-native-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(infra_core::fs::canonicalize(path).unwrap())
    }
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write(root: &Path, name: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn document(root: &Path, name: &str, value: &Value) {
    write(root, name, serde_json::to_vec_pretty(value).unwrap());
}
fn passport() -> Value {
    json!({"schema_version":1,"id":"payments","description":"Payment service","owner":"unknown","relationships_complete":true,"capabilities":[{"id":"payments","description":"Payment operations","status":"implemented","sources":["src/payments.txt"],"keywords":["payments"]}],"provides":[],"consumes":[],"sources":[{"path":"src/payments.txt","kind":"code"}]})
}
fn registry(root: impl Into<Value>) -> Value {
    let root: Value = root.into();
    json!({"schema_version":1,"name":"Example system","services":[{"id":"payments","root":root,"manifest":"ai-service.json"}],"shared_sources":[]})
}
fn fixture(temp: &Temp) -> (PathBuf, PathBuf, PathBuf) {
    let control = temp.dir("control");
    let service = control.join("service");
    fs::create_dir(&service).unwrap();
    write(
        &service,
        "src/payments.txt",
        "Payment operations fixture only.\n",
    );
    document(&service, "ai-service.json", &passport());
    document(&control, "system.json", &registry("service"));
    let config = control.join("system.json");
    (control, service, config)
}
fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for item in fs::read_dir(path).unwrap() {
            let item = item.unwrap();
            let path = item.path();
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(
                !meta.file_type().is_symlink(),
                "test fixture contains a link"
            );
            if meta.is_dir() {
                out.insert(format!("{name}/"), None);
                visit(root, &path, out);
            } else {
                out.insert(name, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn cli_value(value: &Value) -> Value {
    assert_eq!(value["_cli"]["code"], 0, "{}", value["_cli"]["stderr"]);
    serde_json::from_str(value["_cli"]["stdout"].as_str().unwrap()).unwrap()
}

#[test]
fn validated_plans_are_read_only_and_do_not_discover_or_execute_providers() {
    let temp = Temp::new();
    let (control, service, config) = fixture(&temp);
    let before = tree(&temp.0);
    let mut system = System::new(&config, &[]).unwrap();
    assert!(system.warnings.is_empty());
    let located = system.locate("payments").unwrap();
    assert_eq!(located["candidates"][0]["service"], "payments");
    let plan = system
        .plan(
            "Change payments",
            "change-1",
            &["payments".into()],
            &[],
            8000,
            32,
        )
        .unwrap();
    assert_eq!(plan["status"], "needs_review");
    assert_eq!(plan["executed"], false);
    assert_eq!(
        plan["context"]["sources"][0]["excerpt"],
        "Payment operations fixture only.\n"
    );
    assert!(plan["context_chars"].as_u64().unwrap() <= 8000);
    assert_eq!(plan["memory_layout"]["services"][0]["tasks_created"], false);
    assert!(
        plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|step| step.get("task_reference").is_some())
            .all(|step| step["task_reference"]["created"] == false)
    );
    assert_eq!(system.verify(&plan).unwrap()["fresh"], true);
    // Even unusable execution options must be irrelevant to read-only routes.
    let request = json!({"system":config,"task":"Change payments","change_id":"change-2","service":["payments"],"context_budget":8000,"provider":"must-not-discover","executable":control.join("missing-provider")});
    let from_cli = cli_value(&execute("plan", &request).unwrap());
    assert_eq!(from_cli["executed"], false);
    assert_eq!(
        cli_value(&execute("validate", &request).unwrap())["valid"],
        true
    );
    assert_eq!(
        tree(&temp.0),
        before,
        "planning/validation mutated a project or system root"
    );
    assert!(!service.join("project-brain").exists());
    assert!(!control.join("project-brain").exists());
}
#[test]
fn saved_plan_freshness_checks_content_and_newly_appeared_sources() {
    let temp = Temp::new();
    let (_control, service, config) = fixture(&temp);
    let mut declared = passport();
    declared["sources"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":"src/missing.txt","kind":"code"}));
    document(&service, "ai-service.json", &declared);
    let mut system = System::new(&config, &[]).unwrap();
    let plan = system
        .plan(
            "Change payments",
            "change-1",
            &["payments".into()],
            &[],
            8000,
            32,
        )
        .unwrap();
    assert!(
        plan["snapshot"]["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["service"] == "payments" && item["path"] == "src/missing.txt")
    );
    assert_eq!(system.verify(&plan).unwrap()["fresh"], true);
    write(
        &service,
        "src/payments.txt",
        "Changed payment fixture text.\n",
    );
    write(&service, "src/missing.txt", "A newly available source.\n");
    let mut current = System::new(&config, &[]).unwrap();
    let stale = current.verify(&plan).unwrap();
    assert_eq!(stale["fresh"], false);
    let changes = stale["changed"].as_array().unwrap();
    assert!(
        changes.iter().any(
            |entry| entry["path"] == "src/payments.txt" && entry["reason"] == "content_changed"
        )
    );
    assert!(changes.iter().any(|entry|entry["path"]=="src/missing.txt"&&entry["reason"]=="source_appeared"));
}
#[test]
fn registry_cannot_grant_itself_external_root_access() {
    let temp = Temp::new();
    let control = temp.dir("control");
    let external = temp.dir("external-service");
    write(
        &external,
        "src/payments.txt",
        "External service evidence.\n",
    );
    document(&external, "ai-service.json", &passport());
    document(&control, "system.json", &registry(json!(external)));
    let config = control.join("system.json");
    let before = tree(&temp.0);
    let denied = System::new(&config, &[]).unwrap();
    assert_eq!(denied.services["payments"].access, "denied");
    assert!(denied.services["payments"].manifest.is_none());
    assert!(
        denied
            .warnings
            .iter()
            .any(|warning| warning["reason"] == "root_not_allowed")
    );
    assert!(
        !denied.snapshot.keys().any(|(sid, _)| sid == "payments"),
        "denied service files were read"
    );
    let admitted = System::new(&config, std::slice::from_ref(&external)).unwrap();
    assert_eq!(admitted.services["payments"].access, "available");
    assert!(
        admitted
            .snapshot
            .contains_key(&("payments".into(), "ai-service.json".into()))
    );
    assert_eq!(tree(&temp.0), before);
}
#[test]
fn unsafe_source_paths_and_oversized_registries_are_refused() {
    let temp = Temp::new();
    let (_control, service, config) = fixture(&temp);
    for path in [
        "../outside.txt",
        ".env",
        "vendor/package.php",
        "project-brain/local/context.db",
        "src\\payments.txt",
    ] {
        let mut declared = passport();
        declared["sources"][0]["path"] = json!(path);
        document(&service, "ai-service.json", &declared);
        assert!(
            System::new(&config, &[]).is_err(),
            "unsafe source path accepted: {path}"
        );
    }
    let mut declared = passport();
    declared["sources"][0]["path"] = json!(format!("src/{}.txt", "a".repeat(1024)));
    document(&service, "ai-service.json", &declared);
    assert!(System::new(&config, &[]).is_err());
    let mut data = registry("service");
    data["services"]=Value::Array((0..=common::MAX_SERVICES).map(|index|json!({"id":format!("service-{index}"),"root":"service","manifest":"ai-service.json"})).collect());
    document(config.parent().unwrap(), "system.json", &data);
    assert!(System::new(&config, &[]).is_err());
}
#[test]
fn context_and_source_byte_budgets_apply_before_unbounded_admission() {
    let temp = Temp::new();
    let (_control, service, config) = fixture(&temp);
    let mut system = System::new(&config, &[]).unwrap();
    let small = system
        .plan("payments", "change-1", &["payments".into()], &[], 128, 0)
        .unwrap();
    assert!(small["context_chars"].as_u64().unwrap() <= 128);
    assert!(small["context"]["sources"].as_array().unwrap().is_empty());
    assert!(
        small["omitted_sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["reason"] == "budget")
    );
    assert!(
        system
            .plan("payments", "change-1", &["payments".into()], &[], 127, 0)
            .is_err()
    );
    assert!(
        system
            .plan("payments", "change-1", &["payments".into()], &[], 64001, 0)
            .is_err()
    );
    assert!(
        system
            .plan("payments", "change-1", &["payments".into()], &[], 8000, 65)
            .is_err()
    );
    write(
        &service,
        "src/large.txt",
        vec![b'a'; common::MAX_SOURCE_BYTES + 1],
    );
    let mut declared = passport();
    declared["capabilities"][0]["sources"] = json!(["src/large.txt"]);
    declared["sources"] = json!([{"path":"src/large.txt","kind":"code"}]);
    document(&service, "ai-service.json", &declared);
    let mut bounded = System::new(&config, &[]).unwrap();
    assert!(
        bounded
            .read("payments", &service, "src/large.txt", None, None)
            .is_err()
    );
    assert!(
        !bounded
            .snapshot
            .contains_key(&("payments".into(), "src/large.txt".into()))
    );
    let plan = bounded
        .plan("payments", "change-2", &["payments".into()], &[], 64000, 32)
        .unwrap();
    assert!(plan["context"]["sources"].as_array().unwrap().is_empty());
    assert!(
        plan["omitted_sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "src/large.txt" && entry["reason"] == "ineligible")
    );
    assert!(plan["context_chars"].as_u64().unwrap() <= 64000);
}
#[test]
fn malformed_system_json_never_becomes_a_valid_catalog() {
    let temp = Temp::new();
    let control = temp.dir("control");
    let config = control.join("system.json");
    assert!(common::parse_json(br#"{"number":123456789012345678901}"#, common::MAX_BYTES).is_err());
    for raw in [
        r#"{"schema_version":1,"schema_version":1,"name":"Example","services":[],"shared_sources":[]}"#,
        r#"{"schema_version":1,"name":"Example","services":[],"shared_sources":[],"unexpected":true}"#,
        r#"{"schema_version":NaN,"name":"Example","services":[],"shared_sources":[]}"#,
        r#"{"schema_version":1,"name":"Example","services":[],"shared_sources":[],"number":123456789012345678901}"#,
    ] {
        write(&control, "system.json", raw);
        assert!(
            System::new(&config, &[]).is_err(),
            "malformed or unbounded catalog accepted"
        );
    }
}
#[cfg(unix)]
#[test]
fn declared_source_links_are_not_followed_into_an_outside_file() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let (_control, service, config) = fixture(&temp);
    write(
        &temp.0,
        "outside.txt",
        "Outside evidence must not enter the plan.\n",
    );
    fs::remove_file(service.join("src/payments.txt")).unwrap();
    symlink(temp.0.join("outside.txt"), service.join("src/payments.txt")).unwrap();
    let mut system = System::new(&config, &[]).unwrap();
    let plan = system
        .plan("payments", "change-1", &["payments".into()], &[], 8000, 32)
        .unwrap();
    assert!(plan["context"]["sources"].as_array().unwrap().is_empty());
    assert!(
        plan["omitted_sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["reason"] == "ineligible")
    );
    assert!(
        !system
            .snapshot
            .contains_key(&("payments".into(), "src/payments.txt".into()))
    );
}
