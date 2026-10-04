//! ModelSpec serialization behavior tests.

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

// ④ error path: malformed JSON (truncated / not JSON at all)
// 断言错误类别（serde_json::Error::classify），而非恒真的「非空」检查。
#[test]
fn test_model_spec_deserialize_malformed_json_errors() {
    use serde_json::error::Category;
    for (input, expected) in [
        ("", Category::Eof),
        (r#"{"primary":"a/b""#, Category::Eof),
        ("not json", Category::Syntax),
        ("{", Category::Eof),
        // 序列形态：Visitor 未实现 visit_seq，类型错在截断语法错被检出前先返回
        (r#"["a/b""#, Category::Data),
    ] {
        let err = serde_json::from_str::<ModelSpec>(input).expect_err("malformed input must fail");
        let actual = err.classify();
        assert_eq!(
            actual, expected,
            "input {input:?}: expected error category {expected:?}, got {actual:?}: {err}"
        );
    }
}

// ④ error path: `fallback` field has wrong value type in object form
#[test]
fn test_model_spec_deserialize_fallback_wrong_type_errors() {
    let err = serde_json::from_str::<ModelSpec>(r#"{"primary":"a/b","fallback":"x/y"}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid type"), "unexpected: {err}");
    let err = serde_json::from_str::<ModelSpec>(r#"{"primary":"a/b","fallback":[42]}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid type"), "unexpected: {err}");
}

// ⑥ constructor boundary: with_fallback with empty list == single (serializes as string form)
#[test]
fn test_model_spec_with_fallback_empty_list_equals_single() {
    let spec = ModelSpec::with_fallback("provider/model", Vec::new());
    assert_eq!(spec, ModelSpec::single("provider/model"));
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(json, r#""provider/model""#);
    assert_eq!(serde_json::from_str::<ModelSpec>(&json).unwrap(), spec);
}

// ⑥ constructor boundary: with_fallback keeps multiple fallbacks in order through roundtrip
#[test]
fn test_model_spec_with_fallback_multiple_preserves_order() {
    let spec = ModelSpec::with_fallback(
        "provider/model",
        vec!["a/a".into(), "b/b".into(), "c/c".into()],
    );
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(
        json,
        r#"{"primary":"provider/model","fallback":["a/a","b/b","c/c"]}"#
    );
    let back = serde_json::from_str::<ModelSpec>(&json).unwrap();
    assert_eq!(back, spec);
    assert_eq!(back.fallback, vec!["a/a", "b/b", "c/c"]);
    assert_eq!(back.to_string(), "provider/model");
}
