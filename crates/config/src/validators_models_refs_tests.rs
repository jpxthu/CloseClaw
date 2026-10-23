//! Tests for models `credentialPath` file existence and `apiKey` credential
//! cross-reference validation.

use std::collections::HashSet;

use crate::validators::{validate_models, validate_models_with_refs, CredentialProviderSet};

// ---------------------------------------------------------------------------
// Step 1.4 — models credentialPath file existence validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_credential_path_exists() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let json = format!(
        r#"{{"providers":{{"p":{{"credentialPath":"{}","models":[]}}}}}}"#,
        path
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(validate_models(&v).is_ok());
}

#[test]
fn test_validate_models_credential_path_not_exists() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"providers":{"p":{"credentialPath":"/nonexistent/path/to/cred","models":[]}}}"#,
    )
    .unwrap();
    let err = validate_models(&v).unwrap_err();
    assert!(err.contains("does not exist"), "error: {}", err);
    assert!(
        err.contains("credentialPath"),
        "error should mention credentialPath: {}",
        err
    );
}

#[test]
fn test_validate_models_credential_path_null_or_absent() {
    let v_null: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"credentialPath":null,"models":[]}}}"#).unwrap();
    assert!(validate_models(&v_null).is_ok());
    let v_absent: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"p":{"models":[]}}}"#).unwrap();
    assert!(validate_models(&v_absent).is_ok());
}

// ---------------------------------------------------------------------------
// Step 1.3 — models apiKey credential cross-validation tests
// ---------------------------------------------------------------------------

#[test]
fn test_validate_models_api_key_ref_unknown_cred_provider() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"apiKey":"sk-test","models":[]}}}"#)
            .unwrap();
    let crps = CredentialProviderSet {
        names: ["anthropic".to_string()].into_iter().collect(),
    };
    let err = validate_models_with_refs(&v, Some(&crps)).unwrap_err();
    assert!(
        err.contains("references an unknown credential provider"),
        "error: {}",
        err
    );
    assert!(
        err.contains("openai"),
        "error should mention openai: {}",
        err
    );
}

#[test]
fn test_validate_models_api_key_ref_known_cred_provider() {
    let v: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"apiKey":"sk-test","models":[]}}}"#)
            .unwrap();
    let crps = CredentialProviderSet {
        names: ["openai".into(), "anthropic".into()].into_iter().collect(),
    };
    assert!(validate_models_with_refs(&v, Some(&crps)).is_ok());
}

#[test]
fn test_validate_models_pass_no_api_key_or_credential_path() {
    let empty_creds = CredentialProviderSet {
        names: HashSet::new(),
    };
    // No apiKey: passes regardless of credential providers
    let v1: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"models":[{"id":"gpt-4"}]}}}"#).unwrap();
    assert!(validate_models_with_refs(&v1, Some(&empty_creds)).is_ok());
    // No cross-ref: skips credential check entirely
    let v2: serde_json::Value =
        serde_json::from_str(r#"{"providers":{"openai":{"apiKey":"sk-test","models":[]}}}"#)
            .unwrap();
    assert!(validate_models_with_refs(&v2, None).is_ok());
}

#[test]
fn test_validate_models_api_key_with_credential_path_passes() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let json = format!(
        r#"{{"providers":{{"openai":{{"apiKey":"sk-test","credentialPath":"{}","models":[]}}}}}}"#,
        tmp.path().to_str().unwrap()
    );
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(validate_models_with_refs(
        &v,
        Some(&CredentialProviderSet {
            names: HashSet::new()
        })
    )
    .is_ok());
}
