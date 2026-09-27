//! ModelSpec serialization behavior tests.
//!
//! Split from `tests.rs` to keep both files below the 1000-line limit.

use super::*;

// ── ModelSpec tests: ① string form + Display ② object form ③ roundtrip ④ errors ──

#[test]
fn test_model_spec_deserialize_string_form_and_display() {
    let spec: ModelSpec = serde_json::from_str(r#""provider/model""#).unwrap();
    assert_eq!(spec.primary, "provider/model");
    assert!(spec.fallback.is_empty());
    assert_eq!(spec.to_string(), "provider/model");
}

// ② object form {primary, fallback} deserializes; absent fallback stays empty
#[test]
fn test_model_spec_deserialize_object_form() {
    let spec: ModelSpec =
        serde_json::from_str(r#"{"primary":"a/b","fallback":["x","y"]}"#).unwrap();
    assert_eq!(spec.primary, "a/b");
    assert_eq!(spec.fallback, vec!["x", "y"]);
    assert_eq!(spec.to_string(), "a/b");
    let no_fb: ModelSpec = serde_json::from_str(r#"{"primary": "provider/model"}"#).unwrap();
    assert_eq!(no_fb, ModelSpec::single("provider/model"));
}

// ③ serialize→deserialize roundtrip: no fallback → string form, with fallback → object form
#[test]
fn test_model_spec_roundtrip_single_string_form() {
    let spec = ModelSpec::single("provider/model");
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(json, r#""provider/model""#);
    assert_eq!(serde_json::from_str::<ModelSpec>(&json).unwrap(), spec);
}

#[test]
fn test_model_spec_roundtrip_with_fallback_object_form() {
    let spec = ModelSpec::with_fallback("provider/model", vec!["fallback/a".into()]);
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(
        json,
        r#"{"primary":"provider/model","fallback":["fallback/a"]}"#
    );
    assert_eq!(serde_json::from_str::<ModelSpec>(&json).unwrap(), spec);
}

// ④ error paths: missing `primary` field, and non-string/non-object forms
#[test]
fn test_model_spec_deserialize_missing_primary_errors() {
    let err = serde_json::from_str::<ModelSpec>(r#"{"fallback":["a/b"]}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("missing field `primary`"), "unexpected: {err}");
}

#[test]
fn test_model_spec_deserialize_non_string_object_errors() {
    for input in ["42", "true", "null", r#"["a/b"]"#] {
        let err = serde_json::from_str::<ModelSpec>(input)
            .expect_err("invalid form must fail")
            .to_string();
        assert!(err.contains("invalid type"), "input {input}: {err}");
    }
}

// ④ boundary: empty string accepted as-is (no content validation)
#[test]
fn test_model_spec_deserialize_empty_string_accepted_as_is() {
    let spec: ModelSpec = serde_json::from_str(r#""""#).unwrap();
    assert_eq!(spec, ModelSpec::single(""));
}
// ⑤ visit_map: duplicate `primary` key is rejected
#[test]
fn test_model_spec_deserialize_duplicate_primary_errors() {
    let err = serde_json::from_str::<ModelSpec>(r#"{"primary":"a/b","primary":"c/d"}"#)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("duplicate field `primary`"),
        "unexpected: {err}"
    );
}

// ⑤ visit_map: duplicate `fallback` key is rejected
#[test]
fn test_model_spec_deserialize_duplicate_fallback_errors() {
    let err = serde_json::from_str::<ModelSpec>(
        r#"{"primary":"a/b","fallback":["x/y"],"fallback":["z/w"]}"#,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("duplicate field `fallback`"),
        "unexpected: {err}"
    );
}

// ⑤ visit_map: unknown fields are ignored (IgnoredAny)
#[test]
fn test_model_spec_deserialize_unknown_fields_ignored() {
    let spec: ModelSpec =
        serde_json::from_str(r#"{"primary":"a/b","fallback":["x/y"],"extra":"ignored"}"#).unwrap();
    assert_eq!(spec.primary, "a/b");
    assert_eq!(spec.fallback, vec!["x/y"]);
    let with_object_extra: ModelSpec =
        serde_json::from_str(r#"{"primary":"a/b","unknown":{"nested":1}}"#).unwrap();
    assert_eq!(with_object_extra, ModelSpec::single("a/b"));
}
