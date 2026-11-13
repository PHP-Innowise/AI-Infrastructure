//! Public evaluator JSON contracts must survive executable-wide serde feature unification.
use infra_eval::artifact;
use serde_json::{Value, json};

#[test]
fn finite_fractional_and_exponent_tokens_remain_numbers() {
    for (raw, expected) in [
        ("0.1", 0.1),
        ("-1.25", -1.25),
        ("1e-3", 0.001),
        ("6.022e23", 6.022e23),
    ] {
        let value = artifact::strict(raw.as_bytes()).unwrap();
        assert!(value.is_number(), "{raw} decoded as {value:?}");
        assert_eq!(value.as_f64(), Some(expected), "{raw}");
    }
}

#[test]
fn nested_outcome_design_decimals_keep_their_numeric_type() {
    let value = artifact::strict(
        br#"{"quality_margin":0.1,"nested":[{"ratio":-1.25},2e3],"enabled":true,"nothing":null}"#,
    )
    .unwrap();
    assert_eq!(value["quality_margin"].as_f64(), Some(0.1));
    assert_eq!(value["nested"][0]["ratio"].as_f64(), Some(-1.25));
    assert_eq!(value["nested"][1].as_f64(), Some(2000.0));
    assert_eq!(value["enabled"], true);
    assert!(value["nothing"].is_null());
}

#[test]
fn duplicate_decoded_keys_are_refused_at_every_object_depth() {
    for raw in [
        r#"{"key":1,"key":2}"#,
        r#"{"a":1,"\u0061":2}"#,
        r#"{"outer":[{"key":1,"key":2}]}"#,
        r#"{"$serde_json::private::Number":"0.1","$serde_json::private::Number":"0.2"}"#,
    ] {
        assert!(artifact::strict(raw.as_bytes()).is_err(), "{raw}");
    }
}

#[test]
fn actual_reserved_key_objects_are_never_reinterpreted_as_numbers() {
    let raw = br#"{"$serde_json::private::Number":"0.1"}"#;
    let value = artifact::strict(raw).unwrap();
    assert!(value.is_object());
    assert_eq!(value.as_object().unwrap().len(), 1);
    assert_eq!(value["$serde_json::private::Number"], "0.1");
    let nested = artifact::strict(
        br#"{"outer":[{"$serde_json::private::Number":"1e400"},{"$serde_json::private::RawValue":"null"}]}"#,
    )
    .unwrap();
    assert_eq!(
        nested,
        json!({"outer":[
            {"$serde_json::private::Number":"1e400"},
            {"$serde_json::private::RawValue":"null"}
        ]})
    );
}

#[test]
fn integer_tokens_beyond_fixed_width_and_float_ranges_keep_exact_digits() {
    let huge = format!("1{}", "0".repeat(400));
    for raw in [
        "340282366920938463463374607431768211456",
        "-340282366920938463463374607431768211457",
        huge.as_str(),
    ] {
        let value = artifact::strict(raw.as_bytes()).unwrap();
        let Value::Number(number) = value else {
            panic!("integer token decoded as non-number: {raw}");
        };
        assert_eq!(number.to_string(), raw);
    }
}

#[test]
fn nonfinite_overflow_trailing_and_invalid_encoding_are_refused() {
    for raw in [
        "NaN",
        "Infinity",
        "-Infinity",
        "1e400",
        "-1e400",
        r#"{"nested":[1e400]}"#,
        "{} {}",
        "0.1 trailing",
        "[1,]",
        r#"{"broken":}"#,
    ] {
        assert!(artifact::strict(raw.as_bytes()).is_err(), "{raw}");
    }
    assert!(artifact::strict(&[b'"', 0xff, b'"']).is_err());
}
