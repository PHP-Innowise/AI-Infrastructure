//! The public CLI adapter must preserve raw optional hint JSON until its
//! bounded strict decoder decides whether the hints are valid.
use infra_core::{application::request_retrieval, source_hints};
use serde_json::json;

#[test]
fn source_hint_duplicate_decoded_members_are_invalid_at_the_cli_adapter() {
    for raw in [
        r#"{"schema_version":1,"schema_version":1,"references":[]}"#,
        r#"{"schema_version":1,"schema\u005fversion":1,"references":[]}"#,
        r#"{"schema_version":1,"references":[],"refer\u0065nces":[]}"#,
    ] {
        let request = request_retrieval(&json!({
            "query":"native hint protocol",
            "source_hints_json":raw
        }))
        .expect("optional malformed hint JSON must not abort the command");
        let (references, state) =
            source_hints::normalize(request.source_hints.as_ref().expect("raw hints retained"));
        assert_eq!(state, "invalid", "{raw}");
        assert!(references.is_empty());
    }
}

#[test]
fn valid_raw_source_hint_profile_is_still_accepted() {
    let request = request_retrieval(&json!({
        "query":"native hint protocol",
        "source_hints_json":r#"{"schema_version":1,"references":[]}"#
    }))
    .expect("valid CLI request");
    let (references, state) = source_hints::normalize(request.source_hints.as_ref().unwrap());
    assert_eq!(state, "ready");
    assert!(references.is_empty());
}
