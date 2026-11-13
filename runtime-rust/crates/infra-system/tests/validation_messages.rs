//! A refused passport says which service, which entry and which rule, so the
//! editor's single error line leads to the field that needs correcting.
use infra_system::catalog::manifest;
use serde_json::{Value, json};

fn passport() -> Value {
    json!({"schema_version":1,"id":"orders","description":"Orders","owner":"Team",
        "capabilities":[{"id":"placement","description":"Place orders","status":"implemented","sources":[]}],
        "provides":[],"consumes":[],"sources":[]})
}
fn refusal(value: &Value) -> String {
    manifest(value, "orders").unwrap_err().message
}

#[test]
fn an_invalid_capability_id_names_its_service_entry_and_rule() {
    let mut value = passport();
    value["capabilities"][0]["id"] = json!("Order Cancel");
    let message = refusal(&value);
    assert!(
        message.starts_with("Service orders: capability 1: Invalid identifier \"Order Cancel\""),
        "{message}"
    );
    assert!(message.contains("lowercase letters"), "{message}");
}

#[test]
fn an_unknown_status_lists_the_allowed_ones() {
    let mut value = passport();
    value["capabilities"][0]["status"] = json!("done");
    let message = refusal(&value);
    assert!(message.contains("capability 1: Invalid capability status \"done\": choose implemented, partial, planned, unknown"), "{message}");
}

#[test]
fn unknown_and_missing_keys_are_named() {
    let mut value = passport();
    value["capabilities"][0]
        .as_object_mut()
        .unwrap()
        .remove("sources");
    value["capabilities"][0]["tags"] = json!([]);
    let message = refusal(&value);
    assert!(message.contains("capability 1: Invalid object fields (missing or unknown keys): missing sources; unknown tags"), "{message}");
}

#[test]
fn a_refused_source_path_is_quoted() {
    let mut value = passport();
    value["sources"] = json!([{"path":"vendor/autoload.php","kind":"code"}]);
    let message = refusal(&value);
    assert!(
        message
            .starts_with("Service orders: source 1: Refused source path \"vendor/autoload.php\""),
        "{message}"
    );
}

#[test]
fn duplicates_name_what_repeats() {
    let mut value = passport();
    value["consumes"] = json!([
        {"service":"catalog","contract":"catalog.products","version":"2"},
        {"service":"catalog","contract":"catalog.products","version":"2"}]);
    let message = refusal(&value);
    assert!(
        message
            .contains("consumed contract 2: Duplicate consumed contract catalog:catalog.products"),
        "{message}"
    );
}
